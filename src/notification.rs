//! In-memory notification lifecycle, ported from Mako's `notification.c`.
//!
//! No SQLite, channels, or other post-MVP behavior belongs in this module.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Urgency { Low, Normal, Critical }

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
    pub progress: u8,
    pub requested_timeout_ms: i32,
    pub actions: Vec<Action>,
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
    pub progress: u8,
    pub requested_timeout_ms: i32,
    pub actions: Vec<Action>,
}

impl Default for Request {
    fn default() -> Self {
        Self { replaces_id: 0, app_name: String::new(), app_icon: String::new(), image_data: None,
            summary: String::new(), body: String::new(), category: String::new(),
            desktop_entry: String::new(), tag: String::new(), urgency: Urgency::Normal,
            progress: 0, requested_timeout_ms: -1, actions: Vec::new() }
    }
}

impl Notification {
    fn from_request(id: u32, request: Request) -> Self {
        Self { id, app_name: request.app_name, app_icon: request.app_icon,
            image_data: request.image_data, summary: request.summary, body: request.body, category: request.category,
            desktop_entry: request.desktop_entry, tag: request.tag, urgency: request.urgency,
            progress: request.progress, requested_timeout_ms: request.requested_timeout_ms,
            actions: request.actions }
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
    max_history: usize,
    sort_policy: SortPolicy,
}

impl NotificationStore {
    pub fn new(max_history: usize) -> Self {
        Self { active: VecDeque::new(), history: VecDeque::new(),
            next_id: 1, max_history, sort_policy: SortPolicy::default() }
    }

    pub fn active(&self) -> &VecDeque<Notification> { &self.active }
    pub fn history(&self) -> &VecDeque<Notification> { &self.history }

    /// Configure sorting before accepting notifications.
    /// Re-sorting existing notifications on config reload is pending.
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
        let rank = |urgency: Urgency| match urgency {
            Urgency::Low => 0u8,
            Urgency::Normal => 1,
            Urgency::Critical => 2,
        };
        let rank_new = rank(notification.urgency);
        let policy = self.sort_policy;
        let index = self.active.iter().position(|existing| {
            let rank_old = rank(existing.urgency);
            if rank_new == rank_old {
                // Newer first for -time, older first for +time.
                !policy.time_ascending
            } else if policy.priority_ascending {
                rank_new < rank_old
            } else {
                rank_new > rank_old
            }
        }).unwrap_or(self.active.len());
        self.active.insert(index, notification);
    }

    pub fn push(&mut self, request: Request) -> u32 {
        // `replaces_id` only refers to currently active notifications.
        let existing = if request.replaces_id != 0 {
            self.active.iter().position(|n| n.id == request.replaces_id)
        } else { None };
        let existing = existing.or_else(|| {
            if request.tag.is_empty() { return None; }
            self.active.iter().position(|n|
                n.app_name == request.app_name && n.tag == request.tag)
        });
        if let Some(index) = existing {
            let id = self.active[index].id;
            self.active[index] = Notification::from_request(id, request);
            return id;
        }
        let id = self.allocate_id();
        self.insert_sorted(Notification::from_request(id, request));
        id
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
        let notification = self.active.remove(idx)?;
        if save_history && history_enabled && self.max_history != 0 {
            self.history.push_front(notification.clone());
            self.history.truncate(self.max_history);
        }
        Some(notification)
    }

    /// Restores the newest notification from the in-memory history.
    pub fn restore(&mut self) -> Option<u32> {
        let notification = self.history.pop_front()?;
        let id = notification.id;
        // Mako reinserts restored notifications using the active sort policy;
        // pushing to the front would break urgency and ascending-time ordering.
        self.insert_sorted(notification);
        Some(id)
    }

    pub fn set_max_history(&mut self, max_history: usize) {
        self.max_history = max_history;
        self.history.truncate(max_history);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(app: &str, text: &str) -> Request {
        Request { app_name: app.into(), summary: text.into(), ..Request::default() }
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
