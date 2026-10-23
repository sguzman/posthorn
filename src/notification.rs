//! In-memory notification lifecycle, ported from Mako's `notification.c`.
//!
//! No SQLite, channels, or other post-MVP behavior belongs in this module.

use std::collections::{HashMap,HashSet,VecDeque};

// Do not derive Ord: variant order would incorrectly place Other(-3) above
// Critical. Sorting must compare mako_value(), the original signed C enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Urgency { Unknown, Low, Normal, Critical, Other(i32) }

impl Urgency {
    /// Mako keeps the signed enum value supplied by y/u/i D-Bus hints, even
    /// when it does not correspond to a recognized urgency. uint32 values
    /// narrow to C's signed enum representation.
    pub fn from_mako_wire(value:Option<i64>)->Self {
        match value.map(|number|number as i32){
            None|Some(-1)=>Self::Unknown,
            Some(0)=>Self::Low,
            Some(1)=>Self::Normal,
            Some(2)=>Self::Critical,
            Some(other)=>Self::Other(other),
        }
    }

    pub fn mako_value(self)->i32{
        match self {
            Self::Unknown=>-1,Self::Low=>0,Self::Normal=>1,
            Self::Critical=>2,Self::Other(value)=>value,
        }
    }

    /// Only low/normal/critical can match user urgency criteria.
    pub fn criteria_label(self)->&'static str{
        match self {
            Self::Unknown=>"unknown",Self::Low=>"low",Self::Normal=>"normal",
            Self::Critical=>"critical",Self::Other(_)=>"invalid",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    ClosedByCall = 3,
    Undefined = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Action {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub image_data: Option<crate::icons::IconPixels>,
    pub summary: String,
    pub body: String,
    pub category: String,
    pub desktop_entry: String,
    pub tag: String,
    pub urgency: Urgency,
    pub progress: i32,
    pub requested_timeout_ms: i32,
    pub actions: Vec<Action>,
    /// Group membership assigned by Mako's insertion/close passes.
    pub group_index:i32,
    pub group_count:usize,
    /// Direct NotificationStore fixtures may be unscheduled; render those
    /// with the compatibility replay rather than claiming stored grouping.
    pub grouping_applied:bool,
    /// Mako criteria evaluate [anchor] and [output] only when notif->surface
    /// exists. A fresh notification has no surface during its first pass.
    pub surface_associated:bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub replaces_id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub image_data: Option<crate::icons::IconPixels>,
    pub summary: String,
    pub body: String,
    pub category: String,
    pub desktop_entry: String,
    pub tag: String,
    pub urgency: Urgency,
    pub progress: i32,
    pub requested_timeout_ms: i32,
    pub actions: Vec<Action>,
}

impl Default for Request {
    fn default() -> Self {
        Self { replaces_id: 0, app_name: String::new(), app_icon: String::new(), image_data: None,
            summary: String::new(), body: String::new(), category: String::new(),
            desktop_entry: String::new(), tag: String::new(), urgency: Urgency::Unknown,
            progress: -1, requested_timeout_ms: -1, actions: Vec::new() }
    }
}

impl Notification {
    fn from_request(id: u32, request: Request) -> Self {
        Self { id, app_name: request.app_name, app_icon: request.app_icon,
            image_data: request.image_data, summary: request.summary, body: request.body, category: request.category,
            desktop_entry: request.desktop_entry, tag: request.tag, urgency: request.urgency,
            progress: request.progress, requested_timeout_ms: request.requested_timeout_ms,
            actions: request.actions,
            group_index:-1,group_count:1,grouping_applied:false,
            surface_associated:false }
    }

    /// Returns None for a persistent notification, otherwise its timeout.
    pub fn timeout_ms(&self, default_ms: u32, ignore_sender: bool) -> Option<u32> {
        let timeout = if ignore_sender || self.requested_timeout_ms < 0 {
            default_ms
        } else { self.requested_timeout_ms as u32 };
        if timeout == 0 { None } else { Some(timeout) }
    }
}

/// Mako sort settings. Time breaks urgency ties according to arrival order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortPolicy {
    pub by_priority: bool,
    pub priority_ascending: bool,
    pub time_ascending: bool,
}

impl Default for SortPolicy {
    fn default() -> Self {
        Self { by_priority: false, priority_ascending: false, time_ascending: false }
    }
}

impl SortPolicy {
    pub fn from_config(tokens: &[String]) -> Self {
        let mut policy = Self::default();
        for token in tokens {
            match token.as_str() {
                "+priority" => { policy.by_priority = true; policy.priority_ascending = true; },
                "-priority" => { policy.by_priority = true; policy.priority_ascending = false; },
                "+time" => policy.time_ascending = true,
                "-time" => policy.time_ascending = false,
                _ => {}
            }
        }
        policy
    }
}

/// Mirrors Mako's bounded in-memory history and active list.
#[derive(Debug)]
pub struct NotificationStore {
    active: VecDeque<Notification>,
    history: VecDeque<Notification>,
    next_id: u32,
    // A Freedesktop ID survives Notify replacement and history restoration.
    // Track each active incarnation independently so asynchronous compositor
    // events cannot execute against a newer payload with the same ID.
    next_instance: u64,
    active_instances: HashMap<u32,u64>,
    max_history: usize,
    sort_policy: SortPolicy,
}

impl NotificationStore {
    pub fn new(max_history: usize) -> Self {
        Self { active: VecDeque::new(), history: VecDeque::new(),
            next_id: 1, next_instance: 0, active_instances: HashMap::new(),
            max_history, sort_policy: SortPolicy::default() }
    }

    pub fn active(&self) -> &VecDeque<Notification> { &self.active }

    /// apply_each_criteria creates or reuses a surface after style selection.
    pub fn associate_surface(&mut self,id:u32) {
        if let Some(note)=self.active.iter_mut().find(|n|n.id==id){
            note.surface_associated=true;
        }
    }
    pub fn history(&self) -> &VecDeque<Notification> { &self.history }

    /// Instance numbers are internal only: notification identity on D-Bus
    /// remains the original u32 ID, including explicit/tag replacements.
    pub fn active_instance(&self,id:u32)->Option<u64>{
        self.active_instances.get(&id).copied()
    }

    fn record_new_instance(&mut self,id:u32){
        self.next_instance=self.next_instance.wrapping_add(1);
        if self.next_instance==0{self.next_instance=1;}
        self.active_instances.insert(id,self.next_instance);
    }

    /// Configure sorting before accepting notifications.
    /// Mako reload preserves existing list order; new arrivals use the
    /// updated sort policy, while grouping may still reorder active entries.
    pub fn set_sort_policy(&mut self, policy: SortPolicy) {
        self.sort_policy = policy;
    }

    fn insert_sorted(&mut self, notification: Notification) {
        if !self.sort_policy.by_priority {
            if self.sort_policy.time_ascending {
                self.active.push_back(notification);
            } else {
                self.active.push_front(notification);
            }
            return;
        }
        // Port Mako notification.c:get_last_notif_by_urgency(), not a
        // generic numeric sort. It searches only the known C enum band
        // [-1,2], backwards through the active list, starting from
        // (new urgency + time-direction offset). An out-of-range urgency
        // can therefore insert at the front even for ascending priority.
        // This behavior is observable for D-Bus senders using raw y/u/i
        // values outside the freedesktop 0/1/2 urgency convention.
        let direction:i32=if self.sort_policy.priority_ascending{-1}else{1};
        let offset=if self.sort_policy.time_ascending{0}else{direction};
        let start=notification.urgency.mako_value().checked_add(offset);
        if let Some(mut current)=start {
            while (-1..=2).contains(&current) {
                if let Some(index)=self.active.iter().rposition(|existing|
                    existing.urgency.mako_value()==current
                ){
                    self.active.insert(index+1,notification);
                    return;
                }
                current+=direction;
            }
        }
        self.active.push_front(notification);
    }

    pub fn push(&mut self, request: Request) -> u32 {
        // `replaces_id` only refers to currently active notifications.
        let direct_replace = if request.replaces_id != 0 {
            self.active.iter().position(|n|n.id==request.replaces_id)
        } else {None};
        // Mako handles an explicit existing ID by reset_notification,
        // retaining its group index/count and position. A matching stack
        // tag instead creates a new notification at the old list position.
        // That new object begins ungrouped, even though it inherits the ID.
        if let Some(index)=direct_replace {
            let id=self.active[index].id;
            let old=&self.active[index];
            let (group_index,group_count,grouping_applied,surface_associated)=
                (old.group_index,old.group_count,old.grouping_applied,
                 old.surface_associated);
            self.active[index]=Notification::from_request(id,request);
            self.active[index].group_index=group_index;
            self.active[index].group_count=group_count;
            self.active[index].grouping_applied=grouping_applied;
            // reset_notification leaves the prior surface association.
            self.active[index].surface_associated=surface_associated;
            self.record_new_instance(id);
            return id;
        }
        let tag_replace=if request.tag.is_empty(){None}else{
            self.active.iter().position(|n|
                n.app_name==request.app_name && n.tag==request.tag)
        };
        if let Some(index)=tag_replace {
            // Mako v1.11 dbus/xdg.c creates a new notification before
            // searching for a matching tag. create_notification increments
            // state->last_id even when the tag then reuses the prior ID.
            // Consume that unused ID so the next ordinary Notify agrees.
            let _discarded=self.allocate_id();
            let id=self.active[index].id;
            self.active[index]=Notification::from_request(id,request);
            self.record_new_instance(id);
            return id;
        }
        let id = self.allocate_id();
        self.insert_sorted(Notification::from_request(id, request));
        self.record_new_instance(id);
        id
    }

    /// Match Mako's group_notifications side effect: move the members
    /// matching the newly received notification next to the first match,
    /// while preserving relative sort order among matching and nonmatching
    /// notifications. This is observable through ListNotifications.
    pub fn clear_recorded_grouping(&mut self){
        for note in &mut self.active {
            note.group_index=-1;
            note.group_count=1;
            note.grouping_applied=false;
        }
    }

    pub fn regroup_new_notification(
        &mut self,target_id:u32,spec:&crate::grouping::GroupSpec
    ){
        if let Some(target)=self.active.iter_mut().find(|n|n.id==target_id){
            target.grouping_applied=true;
        }
        if let Some(target)=self.active.iter().find(|n|n.id==target_id).cloned(){
            self.regroup_matching(&target,spec);
        }
    }

    /// Closing a Mako group member re-runs the removed notification's
    /// grouping criteria against the remaining active list.
    pub fn regroup_after_close(
        &mut self,closed:&Notification,spec:&crate::grouping::GroupSpec
    ){
        self.regroup_matching(closed,spec);
    }

    fn regroup_matching(
        &mut self,target:&Notification,spec:&crate::grouping::GroupSpec
    ){
        if spec.disabled{return;}
        let selected:HashSet<u32>=self.active.iter()
            .filter(|note|crate::grouping::same_group(note,target,spec))
            .map(|note|note.id).collect();
        if selected.is_empty(){return;}
        let count=selected.len();
        if count==1 {
            if let Some(note)=self.active.iter_mut()
                .find(|note|selected.contains(&note.id)){
                note.group_index=-1;
                note.group_count=1;
            }
            return;
        }
        let mut matched=VecDeque::new();
        let mut others=VecDeque::new();
        let mut before_first=0usize;
        for notification in self.active.drain(..) {
            if selected.contains(&notification.id){
                matched.push_back(notification);
            }else{
                if matched.is_empty(){before_first+=1;}
                others.push_back(notification);
            }
        }
        for (offset,mut notification) in matched.into_iter().enumerate(){
            notification.group_index=offset as i32;
            notification.group_count=count;
            others.insert(before_first+offset,notification);
        }
        self.active=others;
    }

    fn allocate_id(&mut self) -> u32 {
        // D-Bus notification IDs must never be zero; avoid collisions at wraparound.
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 { self.next_id = 1; }
            if id != 0 && !self.active.iter().any(|n| n.id == id)
                && !self.history.iter().any(|n| n.id == id) { return id; }
        }
    }

    /// Returns closed notification for future D-Bus NotificationClosed emission.
    pub fn close(&mut self, id: u32, _reason: CloseReason, save_history: bool,
                 history_enabled: bool) -> Option<Notification> {
        let idx = self.active.iter().position(|n| n.id == id)?;
        let mut notification = self.active.remove(idx)?;
        self.active_instances.remove(&id);
        // Mako nulls notif->surface when saving a historical notification.
        notification.surface_associated=false;
        if save_history && history_enabled && self.max_history != 0 {
            self.history.push_front(notification.clone());
            self.history.truncate(self.max_history);
        }
        Some(notification)
    }

    /// Restores the newest notification from the in-memory history.
    pub fn restore(&mut self) -> Option<u32> {
        let mut notification = self.history.pop_front()?;
        let id = notification.id;
        // RestoreNotification applies criteria and creates a new surface.
        notification.surface_associated=true;
        // Mako reinserts restored notifications using the active sort policy;
        // pushing to the front would break urgency and ascending-time ordering.
        self.insert_sorted(notification);
        self.record_new_instance(id);
        Some(id)
    }

    /// Mako 1.11's configuration reload replaces the limit but does not
    /// prune the existing history list. Its close_notification() enforces
    /// the new limit only after a history-eligible insertion; at zero it
    /// simply discards the newly closed notification.
    pub fn set_max_history(&mut self, max_history: usize) {
        self.max_history = max_history;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(app: &str, text: &str) -> Request {
        Request { app_name: app.into(), summary: text.into(), ..Request::default() }
    }
    #[test]
    fn regroup_reorders_list_but_never_reverses_matching_members() {
        for time_ascending in [true,false] {
            let mut store=NotificationStore::new(8);
            store.set_sort_policy(SortPolicy{
                by_priority:false,priority_ascending:false,time_ascending
            });
            let a=store.push(Request{app_name:"same".into(),..Request::default()});
            let b=store.push(Request{app_name:"different".into(),..Request::default()});
            let c=store.push(Request{app_name:"same".into(),..Request::default()});
            store.regroup_new_notification(c,
                &crate::grouping::GroupSpec::parse("app-name").unwrap());
            assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
                if time_ascending{vec![a,c,b]}else{vec![c,a,b]});
        }
    }

    #[test]
    fn closing_notification_regroups_matching_remaining_members() {
        let mut store=NotificationStore::new(8);
        let req=|app:&str|Request{app_name:app.into(),..Request::default()};
        let first=store.push(req("same"));
        let middle=store.push(req("other"));
        let third=store.push(req("same"));
        let latest=store.push(req("same"));
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![latest,third,middle,first]);
        let closed=store.close(latest,CloseReason::Dismissed,false,true).unwrap();
        store.regroup_after_close(&closed,
            &crate::grouping::GroupSpec::parse("app-name").unwrap());
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![third,first,middle]);
        assert_eq!(store.active().iter().map(|n|(n.group_index,n.group_count))
            .collect::<Vec<_>>(),vec![(0,2),(1,2),(-1,1)]);
        store.close(third,CloseReason::Dismissed,false,true);
        let last=store.active().iter().find(|n|n.id==first).unwrap().clone();
        store.regroup_after_close(&last,
            &crate::grouping::GroupSpec::parse("app-name").unwrap());
        assert_eq!(store.active().iter().find(|n|n.id==first).unwrap().group_index,-1);
    }

    #[test]
    fn missing_urgency_is_distinct_from_explicit_normal() {
        let mut store=NotificationStore::new(4);
        store.set_sort_policy(SortPolicy{
            by_priority:true,priority_ascending:true,time_ascending:false,
        });
        let normal=store.push(Request{
            urgency:Urgency::Normal,..Request::default()
        });
        let unknown=store.push(Request::default());
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![unknown,normal]);
        assert_eq!(store.active()[0].urgency,Urgency::Unknown);
    }

    #[test]
    fn ids_are_nonzero_and_active_newest_first() {
        let mut store=NotificationStore::new(5);
        let a=store.push(req("a","first"));
        let b=store.push(req("a","second"));
        assert_ne!(a,b);
        assert_eq!(store.active()[0].id,b);
        assert_ne!(store.active()[0].id,0);
    }
    #[test]
    fn progress_defaults_to_mako_unknown_and_stores_raw_sender_value() {
        let mut store=NotificationStore::new(5);
        let id=store.push(Request::default());
        assert_eq!(store.active()[0].progress,-1);
        let newer=Request{replaces_id:id,progress:175,..Request::default()};
        assert_eq!(store.push(newer),id);
        assert_eq!(store.active()[0].progress,175);
    }

    #[test]
    fn replacement_by_id_preserves_id_and_slot() {
        let mut store=NotificationStore::new(5);
        let id=store.push(req("a","old"));
        let mut changed=req("b","new");changed.replaces_id=id;
        assert_eq!(store.push(changed),id);
        assert_eq!(store.active().len(),1);
        assert_eq!(store.active()[0].summary,"new");
    }
    #[test]
    fn tag_replacement_is_scoped_to_application() {
        let mut store=NotificationStore::new(5);
        let mut a=req("one","old");a.tag="progress".into();
        let first=store.push(a);
        let mut b=req("two","other app");b.tag="progress".into();
        assert_ne!(first,store.push(b));
        let mut c=req("one","new");c.tag="progress".into();
        assert_eq!(first,store.push(c));
        assert_eq!(store.active().len(),2);
    }
    #[test]
    fn stack_tag_replacement_consumes_a_fresh_id_but_direct_replacement_does_not() {
        // dbus/xdg.c calls create_notification before discovering a tag
        // match, then rewrites the new object's ID with the old one's ID.
        // That discarded allocation must still advance state->last_id.
        let mut store=NotificationStore::new(5);
        let tagged=Request {
            app_name:"updates".into(),tag:"job".into(),
            ..Request::default()
        };
        let first=store.push(tagged.clone());
        assert_eq!(store.push(tagged.clone()),first);
        assert_eq!(store.push(tagged),first);
        let after_tags=store.push(Request::default());
        assert_eq!(after_tags,first+3,
            "two tag updates must consume two internal notification IDs");

        // Existing-ID replacement invokes reset_notification in place and
        // never creates an object, so it must NOT consume an ID.
        assert_eq!(store.push(Request {
            replaces_id:first, ..Request::default()
        }),first);
        assert_eq!(store.push(Request::default()),after_tags+1);

        // An unmatched nonzero replaces_id does create exactly one object.
        let unmatched=store.push(Request {
            replaces_id:0x7fff_ffff,..Request::default()
        });
        assert_eq!(unmatched,after_tags+2);
    }

    #[test]
    fn instance_identity_changes_on_same_payload_replacement_and_restore(){
        let mut store=NotificationStore::new(5);
        let request=Request{summary:"identical".into(),..Request::default()};
        let id=store.push(request.clone());
        let original=store.active_instance(id).unwrap();
        assert_eq!(store.push(Request{replaces_id:id,..request.clone()}),id);
        let replaced=store.active_instance(id).unwrap();
        assert_ne!(original,replaced,"identical payload is still a new Notify");
        store.close(id,CloseReason::Dismissed,true,true).unwrap();
        assert!(store.active_instance(id).is_none());
        assert_eq!(store.restore(),Some(id));
        assert_ne!(store.active_instance(id),Some(replaced),
            "restored history is a new active lifetime with the same ID");
        let tagged=Request{
            app_name:"sender".into(),tag:"stack-key".into(),
            summary:"same tagged body".into(),..Request::default()
        };
        let tag_id=store.push(tagged.clone());
        let tag_instance=store.active_instance(tag_id).unwrap();
        assert_eq!(store.push(tagged),tag_id,
            "same-sender stack-tag should keep the public notification ID");
        assert_ne!(store.active_instance(tag_id),Some(tag_instance),
            "tag replacement must invalidate a pending compositor click");
    }

    #[test]
    fn explicit_id_replacement_retains_group_metadata_and_list_position() {
        let mut store=NotificationStore::new(8);
        let a=store.push(Request{app_name:"app".into(),..Request::default()});
        // A live notification completes its own initial grouping pass before
        // later messages can group it. Test that lifecycle, not a raw push.
        store.regroup_new_notification(a,
            &crate::grouping::GroupSpec::parse("none").unwrap());
        let unrelated=store.push(Request{app_name:"other".into(),..Request::default()});
        let b=store.push(Request{app_name:"app".into(),..Request::default()});
        store.regroup_new_notification(b,
            &crate::grouping::GroupSpec::parse("app-name").unwrap());
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![b,a,unrelated]);
        let mut replacement=Request{
            replaces_id:a,app_name:"changed".into(),..Request::default()
        };
        replacement.summary="New payload".into();
        assert_eq!(store.push(replacement),a);
        let note=store.active().iter().find(|n|n.id==a).unwrap();
        assert_eq!((note.group_index,note.group_count,note.grouping_applied),(1,2,true));
        assert_eq!(note.summary,"New payload");
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![b,a,unrelated]);
    }

    #[test]
    fn stack_tag_replacement_inherits_id_and_slot_but_not_group_state() {
        let mut store=NotificationStore::new(8);
        let a=store.push(Request{app_name:"app".into(),
            tag:"progress".into(),..Request::default()});
        let unrelated=store.push(Request{app_name:"other".into(),..Request::default()});
        let b=store.push(Request{app_name:"app".into(),..Request::default()});
        store.regroup_new_notification(b,
            &crate::grouping::GroupSpec::parse("app-name").unwrap());
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![b,a,unrelated]);
        let replaced=store.push(Request{app_name:"app".into(),
            tag:"progress".into(),summary:"Updated".into(),..Request::default()});
        assert_eq!(replaced,a);
        let note=store.active().iter().find(|n|n.id==a).unwrap();
        assert_eq!((note.group_index,note.group_count,note.grouping_applied),(-1,1,false));
        assert_eq!(note.summary,"Updated");
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![b,a,unrelated]);
    }

    #[test]
    fn restore_preserves_active_sort_policy() {
        let mut store=NotificationStore::new(4);
        store.set_sort_policy(SortPolicy {
            by_priority:true,priority_ascending:false,time_ascending:false,
        });
        let critical=store.push(Request {
            urgency:Urgency::Critical,..Request::default()
        });
        let low=store.push(Request {
            urgency:Urgency::Low,..Request::default()
        });
        store.close(low,CloseReason::Dismissed,true,true).unwrap();
        assert_eq!(store.restore(),Some(low));
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![critical,low]);
    }

    #[test]
    fn history_is_bounded_and_restore_uses_newest() {
        let mut store=NotificationStore::new(2);
        let a=store.push(req("a","one"));
        let b=store.push(req("a","two"));
        let c=store.push(req("a","three"));
        for id in [a,b,c] { store.close(id,CloseReason::Dismissed,true,true).unwrap(); }
        assert_eq!(store.history().len(),2);
        assert_eq!(store.history()[0].id,c);
        assert_eq!(store.restore(),Some(c));
        assert_eq!(store.active()[0].id,c);
    }
    #[test]
    fn priority_sort_reproduces_mako_bounded_urgency_search(){
        let make=|urgency|Request{urgency,..Request::default()};
        for ascending in [true,false] {
            for time_ascending in [true,false] {
                let mut store=NotificationStore::new(0);
                store.set_sort_policy(SortPolicy{
                    by_priority:true,priority_ascending:ascending,time_ascending,
                });
                // Outside the C enum band, Mako starts at the list sentinel
                // instead of finding a normal numeric insertion position.
                let a=store.push(make(Urgency::Other(3)));
                let b=store.push(make(Urgency::Other(4)));
                let c=store.push(make(Urgency::Other(-3)));
                assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
                    vec![c,b,a],
                    "Mako's bounded search differs from numeric sort");
            }
        }
    }

    #[test]
    fn ascending_priority_keeps_valid_urgencies_in_upstream_order(){
        for time_ascending in [true,false] {
            let mut store=NotificationStore::new(0);
            store.set_sort_policy(SortPolicy{
                by_priority:true,priority_ascending:true,time_ascending,
            });
            let make=|urgency|Request{urgency,..Request::default()};
            let first=store.push(make(Urgency::Normal));
            let second=store.push(make(Urgency::Normal));
            let low=store.push(make(Urgency::Low));
            let critical=store.push(make(Urgency::Critical));
            let unknown=store.push(make(Urgency::Unknown));
            let expected=if time_ascending{
                vec![unknown,low,first,second,critical]
            }else{vec![unknown,low,second,first,critical]};
            assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
                expected);
        }
    }

    #[test]
    fn priority_sort_replays_mako_sentinel_for_unsupported_urgencies(){
        let mut store=NotificationStore::new(5);
        store.set_sort_policy(SortPolicy{
            by_priority:true,priority_ascending:false,time_ascending:false
        });
        let make=|urgency|Request{urgency,..Request::default()};
        let normal=store.push(make(Urgency::Normal));
        let very_low=store.push(make(Urgency::Other(-3)));
        let high=store.push(make(Urgency::Other(7)));
        let critical=store.push(make(Urgency::Critical));
        assert_eq!(store.active().iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![critical,high,very_low,normal],
            "Mako inserts at the head whenever its bounded search finds no recognized urgency");
        assert_eq!(Urgency::from_mako_wire(Some(u32::MAX as i64)),
            Urgency::Unknown);
        assert_eq!(Urgency::Other(-3).mako_value() as u8,253u8);
    }

    #[test]
    fn urgency_sort_preserves_time_order_inside_priority_bands() {
        let mut store = NotificationStore::new(5);
        store.set_sort_policy(SortPolicy {
            by_priority: true, priority_ascending: false, time_ascending: false,
        });
        let mk = |urgency| Request { urgency, ..Request::default() };
        let low = store.push(mk(Urgency::Low));
        let a = store.push(mk(Urgency::Normal));
        let critical = store.push(mk(Urgency::Critical));
        let b = store.push(mk(Urgency::Normal));
        assert_eq!(store.active().iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![critical, b, a, low]);
    }

    #[test]
    fn ascending_time_and_priority_produce_oldest_first_within_band() {
        let mut store = NotificationStore::new(5);
        store.set_sort_policy(SortPolicy {
            by_priority: true, priority_ascending: true, time_ascending: true,
        });
        let a = store.push(Request { urgency: Urgency::Normal, ..Request::default() });
        let b = store.push(Request { urgency: Urgency::Low, ..Request::default() });
        let c = store.push(Request { urgency: Urgency::Normal, ..Request::default() });
        assert_eq!(store.active().iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![b, a, c]);
    }

    #[test]
    fn sort_config_tokens_override_each_dimension() {
        let policy = SortPolicy::from_config(&[
            "-time".into(), "+time".into(), "+priority".into(), "-priority".into()
        ]);
        assert_eq!(policy, SortPolicy {
            by_priority: true, priority_ascending: false, time_ascending: true
        });
    }

    #[test]
    fn no_history_when_disabled() {
        let mut store=NotificationStore::new(3);
        let id=store.push(req("a","one"));
        store.close(id,CloseReason::ClosedByCall,false,true).unwrap();
        assert!(store.history().is_empty());
    }
    #[test]
    fn respects_sender_timeout_unless_ignored() {
        let n=Notification::from_request(7,Request{requested_timeout_ms:9500,..req("x","x")});
        assert_eq!(n.timeout_ms(7000,false),Some(9500));
        assert_eq!(n.timeout_ms(7000,true),Some(7000));
        let n=Notification::from_request(8,Request{requested_timeout_ms:0,..req("x","x")});
        assert_eq!(n.timeout_ms(7000,false),None);
    }
}
