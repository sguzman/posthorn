//! Shared notification lifecycle integration for the staged daemon.
//!
//! The D-Bus adapter and timer driver share this state. No database, channels,
//! or UI code is introduced during the strict Mako MVP port.

use std::collections::HashMap;
use std::path::{Path,PathBuf};
use std::sync::atomic::{AtomicU64,Ordering};
use std::sync::{Arc, Mutex,RwLock};
use std::time::Instant;

use crate::config::{Config, Traits};
use crate::notification::{CloseReason, NotificationStore, SortPolicy};
use crate::hooks::NotifyBinding;
use crate::timers::ExpirationScheduler;

#[derive(Clone, Debug)]
pub struct SharedRuntime {
    pub store: Arc<Mutex<NotificationStore>>,
    timers: Arc<Mutex<ExpirationScheduler>>,
    /// Category/mode-specific history setting captured when a message arrives.
    history_policies: Arc<Mutex<HashMap<u32,bool>>>,
    config: Arc<RwLock<Arc<Config>>>,
    source_path: Option<Arc<PathBuf>>,
    revision: Arc<AtomicU64>,
    modes: Arc<Mutex<Vec<String>>>,
}

impl SharedRuntime {
    pub fn new(config: Config) -> Self {
        let mut store = NotificationStore::new(config.max_history);
        store.set_sort_policy(SortPolicy::from_config(&config.sort));
        Self {
            store: Arc::new(Mutex::new(store)),
            timers: Arc::new(Mutex::new(ExpirationScheduler::default())),
            history_policies: Arc::new(Mutex::new(HashMap::new())),
            config: Arc::new(RwLock::new(Arc::new(config))),
            source_path: None,
            revision: Arc::new(AtomicU64::new(0)),
            modes: Arc::new(Mutex::new(vec!["default".into()])),
        }
    }

    /// Return one immutable configuration snapshot. Readers never see a
    /// partially parsed or partially installed replacement.
    pub fn config(&self)->Arc<Config>{
        self.config.read().expect("config read lock poisoned").clone()
    }

    pub fn revision(&self)->u64{
        self.revision.load(Ordering::Acquire)
    }

    pub fn from_path(path:&Path)->Result<Self,crate::config::ConfigError>{
        let mut runtime=Self::new(Config::from_path(path)?);
        runtime.source_path=Some(Arc::new(path.to_path_buf()));
        Ok(runtime)
    }

    /// Reload only a source-backed instance. Parse before taking the write
    /// lock; invalid or missing files never displace the working config.
    pub fn reload(&self)->Result<(),String>{
        let path=self.source_path.as_ref()
            .ok_or("reload requires a configuration file path")?;
        let updated=Config::from_path(path).map_err(|e|e.to_string())?;
        let sort=SortPolicy::from_config(&updated.sort);
        let max_history=updated.max_history;
        // Store and new configuration are updated under the same gate used
        // by the notification lifecycle. Existing deadlines stay unchanged.
        let mut store=self.store.lock()
            .map_err(|_|"notification store lock poisoned")?;
        let mut active=self.config.write()
            .map_err(|_|"config write lock poisoned")?;
        *active=Arc::new(updated);
        store.set_sort_policy(sort);
        store.set_max_history(max_history);
        drop(active);
        drop(store);
        // Upstream reapply_config rebuilds styles for existing notifications,
        // including the history option. Their original timers are retained.
        self.reapply_active_history_policies()?;
        self.revision.fetch_add(1,Ordering::Release);
        Ok(())
    }

    /// Recalculate active notification history eligibility after Mako-style
    /// mode/config reapplication. This does not reschedule timers or trigger
    /// on-notify bindings, which upstream also leaves alone.
    pub fn reapply_active_history_policies(&self)->Result<(),String>{
        // Keep the store gate until the policy map has been replaced.
        // Otherwise a concurrent Notify could insert its policy between our
        // snapshot and update, only to have the newer ID silently erased.
        let store=self.store.lock()
            .map_err(|_|"notification store lock poisoned")?;
        let modes=self.modes.lock()
            .map_err(|_|"mode state lock poisoned")?.clone();
        let notes=store.active().iter().cloned().collect::<Vec<_>>();
        let groups=crate::grouping::group_ordered_scoped(self,notes.iter(),&modes)?;
        let mut indices=HashMap::new();
        for group in groups {
            for id in &group.ids {
                indices.insert(*id,group.index_of(*id).unwrap_or(-1));
            }
        }
        let mut updated=HashMap::new();
        for note in &notes {
            let index=indices.get(&note.id).copied().unwrap_or(-1);
            let style=crate::desktop::style_for_note_index(self,note,&modes,index);
            let history=style.get("history")
                .is_none_or(|v|v=="1"||v.eq_ignore_ascii_case("true"));
            updated.insert(note.id,history);
        }
        let mut policies=self.history_policies.lock()
            .map_err(|_|"history policy lock poisoned")?;
        *policies=updated;
        Ok(())
    }

    pub fn modes(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.modes)
    }

    /// Schedule the latest snapshot, never an earlier replaced payload.
    /// Lock order is store -> scheduler everywhere to avoid deadlocks.
    pub fn schedule(&self, id: u32, now: Instant) -> Result<(), &'static str> {
        self.schedule_impl(id, now, true)?;
        self.regroup_after_notify(id)?;
        // A newly inserted note may group existing notes; history rules
        // depending on [grouped] / [group-index] must be evaluated afterwards.
        self.reapply_active_history_policies()
            .map_err(|_|"group history style reapplication failed")
    }

    /// Schedule an already-inserted notification without emitting any hook.
    /// This is a test/lifecycle utility, not the history restore operation:
    /// upstream Mako restores historical notifications without new timers.
    pub fn schedule_without_hook(&self, id: u32, now: Instant) -> Result<(), &'static str> {
        self.schedule_impl(id, now, false)?;
        self.regroup_after_notify(id)?;
        self.reapply_active_history_policies()
            .map_err(|_|"group history style reapplication failed")
    }

    fn regroup_after_notify(&self,id:u32)->Result<(), &'static str>{
        let note=self.store.lock().map_err(|_|"notification store lock poisoned")?
            .active().iter().find(|n|n.id==id)
            .ok_or("notification is not active")?.clone();
        let modes=self.modes.lock().map_err(|_|"mode state lock poisoned")?.clone();
        let style=crate::desktop::style_for_note_index(self,&note,&modes,-1);
        let spec=crate::grouping::GroupSpec::parse(
            style.get("group-by").map(String::as_str).unwrap_or("none")
        ).map_err(|_|"invalid group-by style")?;
        self.store.lock().map_err(|_|"notification store lock poisoned")?
            .regroup_new_notification(id,&spec);
        Ok(())
    }

    fn schedule_impl(&self, id: u32, now: Instant, _run_hook: bool) -> Result<(), &'static str> {
        let config=self.config();
        let store = self.store.lock().map_err(|_| "notification store lock poisoned")?;
        let note = store.active().iter().find(|n| n.id == id)
            .ok_or("notification is not active")?;
        let modes = self.modes.lock().map_err(|_| "mode state lock poisoned")?;
        let mode_views: Vec<&str> = modes.iter().map(String::as_str).collect();
        let traits = Traits {
            app_name: &note.app_name, app_icon: &note.app_icon,
            category: &note.category, desktop_entry: &note.desktop_entry,
            summary: &note.summary, body: &note.body,
            urgency: match note.urgency {
                crate::notification::Urgency::Unknown => "unknown",
                crate::notification::Urgency::Low => "low",
                crate::notification::Urgency::Normal => "normal",
                crate::notification::Urgency::Critical => "critical",
            },
            anchor: config.global.get("anchor").map(String::as_str).unwrap_or("top-right"),
            output: config.global.get("output").map(String::as_str).unwrap_or(""),
            modes: &mode_views,
            requested_timeout_ms: note.requested_timeout_ms,
            actionable: !note.actions.is_empty(), group_index: -1, hidden: false,
        };
        let style = config.style_for(&traits);
        let default_ms = style.get("default-timeout")
            .and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
        let ignore_sender = style.get("ignore-timeout")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let timeout = note.timeout_ms(default_ms, ignore_sender);
        self.timers.lock().map_err(|_| "expiration scheduler lock poisoned")?
            .set(id, now, timeout);
        let history=style.get("history")
            .is_none_or(|v|v=="1"||v.eq_ignore_ascii_case("true"));
        self.history_policies.lock().map_err(|_| "history policy lock poisoned")?
            .insert(id,history);
        // Mako executes on-notify AFTER grouping. The Freedesktop D-Bus
        // adapter performs that step after calling this timer/history setup,
        // so all binding kinds observe the same final group-index style.
        Ok(())
    }

    /// Select Mako's on-notify binding after group placement. Returning this
    /// from the runtime leaves all D-Bus signal emission on the async thread.
    pub fn on_notify_binding(&self,id:u32)->Result<NotifyBinding,&'static str>{
        let note=self.store.lock().map_err(|_|"notification state lock poisoned")?
            .active().iter().find(|n|n.id==id)
            .ok_or("notification is not active")?.clone();
        let index=crate::grouping::active_group_index(self,id)
            .map_err(|_|"invalid runtime group-by")?;
        let modes=self.modes.lock().map_err(|_|"mode state lock poisoned")?
            .clone();
        let style=crate::desktop::style_for_note_index(self,&note,&modes,index);
        let raw=style.get("on-notify").map(String::as_str).unwrap_or("none");
        NotifyBinding::parse(raw).map_err(|_|"invalid on-notify binding")
    }

    pub fn close(&self, id: u32, reason: CloseReason, save_history: bool)
        -> Result<bool, &'static str>
    {
        let mut store = self.store.lock().map_err(|_| "notification store lock poisoned")?;
        let mut policies=self.history_policies.lock()
            .map_err(|_| "history policy lock poisoned")?;
        let history_enabled=policies.get(&id).copied().unwrap_or(true);
        let removed=store.close(id, reason, save_history, history_enabled);
        if let Some(note)=removed.as_ref(){
            let modes=self.modes.lock()
                .map_err(|_|"mode state lock poisoned")?.clone();
            let style=crate::desktop::style_for_note_index(self,note,&modes,-1);
            let spec=crate::grouping::GroupSpec::parse(
                style.get("group-by").map(String::as_str).unwrap_or("none")
            ).map_err(|_|"invalid group-by style")?;
            store.regroup_after_close(note,&spec);
        }
        let closed=removed.is_some();
        if closed {
            self.timers.lock().map_err(|_| "expiration scheduler lock poisoned")?
                .cancel(id);
            policies.remove(&id);
        }
        drop(policies);
        drop(store);
        if closed {
            // A surviving member may become ungrouped after its neighbor
            // closes, changing its effective history option.
            self.reapply_active_history_policies()
                .map_err(|_|"group history style reapplication failed")?;
        }
        Ok(closed)
    }

    /// Returns IDs actually expired (already-dismissed timer IDs are ignored).
    pub fn expire(&self, now: Instant) -> Result<Vec<u32>, &'static str> {
        let mut store = self.store.lock().map_err(|_| "notification store lock poisoned")?;
        let expired=self.timers.lock()
            .map_err(|_| "expiration scheduler lock poisoned")?
            .take_expired(now);
        let mut policies=self.history_policies.lock()
            .map_err(|_| "history policy lock poisoned")?;
        let mut closed=Vec::new();
        for id in expired {
            let allow_history=policies.remove(&id).unwrap_or(true);
            if let Some(note)=store.close(id,CloseReason::Expired,true,allow_history) {
                let modes=self.modes.lock()
                    .map_err(|_|"mode state lock poisoned")?.clone();
                let style=crate::desktop::style_for_note_index(self,&note,&modes,-1);
                let spec=crate::grouping::GroupSpec::parse(
                    style.get("group-by").map(String::as_str).unwrap_or("none")
                ).map_err(|_|"invalid group-by style")?;
                store.regroup_after_close(&note,&spec);
                closed.push(id);
            }
        }
        drop(policies);
        drop(store);
        if !closed.is_empty() {
            self.reapply_active_history_policies()
                .map_err(|_|"group history style reapplication failed")?;
        }
        Ok(closed)
    }

    pub fn next_deadline(&self) -> Result<Option<Instant>, &'static str> {
        Ok(self.timers.lock().map_err(|_| "expiration scheduler lock poisoned")?
            .next_deadline())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::{Request, Urgency};
    use std::time::Duration;

    #[test]
    fn category_timeout_overrides_and_replacement_reschedules() {
        let config = Config::parse(
            "default-timeout=7000\nignore-timeout=1\n[category=warning]\ndefault-timeout=2000\n"
        ).unwrap();
        let runtime = SharedRuntime::new(config);
        let first = Instant::now();
        let id = runtime.store.lock().unwrap().push(Request {
            category: "warning".into(), requested_timeout_ms: 50000,
            urgency: Urgency::Critical, ..Request::default()
        });
        runtime.schedule(id, first).unwrap();
        assert!(runtime.expire(first + Duration::from_millis(1900)).unwrap().is_empty());

        runtime.store.lock().unwrap().push(Request {
            replaces_id: id, category: "warning".into(),
            requested_timeout_ms: 99999, ..Request::default()
        });
        let second = first + Duration::from_millis(1950);
        runtime.schedule(id, second).unwrap();
        assert!(runtime.expire(first + Duration::from_millis(2000)).unwrap().is_empty());
        assert_eq!(runtime.expire(second + Duration::from_millis(2000)).unwrap(), vec![id]);
        assert_eq!(runtime.store.lock().unwrap().history().len(), 1);
    }

    #[test]
    fn persistent_and_closed_notifications_have_no_expiration() {
        let runtime = SharedRuntime::new(Config::parse("default-timeout=0\n").unwrap());
        let now = Instant::now();
        let id = runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(id, now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(), None);
        assert!(runtime.close(id, CloseReason::Dismissed, true).unwrap());
        assert!(runtime.expire(now + Duration::from_secs(20)).unwrap().is_empty());
    }

    #[test]
    fn scoped_group_by_moves_active_store_in_mako_sorted_order() {
        for asc in [true,false] {
            let config=Config::parse(&format!(
                "sort={}time\ngroup-by=none\n[category=warning]\ngroup-by=app-name\n",
                if asc{"+"}else{"-"}
            )).unwrap();
            let runtime=SharedRuntime::new(config);
            let now=Instant::now();
            let old=runtime.store.lock().unwrap().push(Request{
                app_name:"same".into(),..Request::default()
            });
            runtime.schedule(old,now).unwrap();
            let unrelated=runtime.store.lock().unwrap().push(Request{
                app_name:"other".into(),..Request::default()
            });
            runtime.schedule(unrelated,now).unwrap();
            let warning=runtime.store.lock().unwrap().push(Request{
                app_name:"same".into(),category:"warning".into(),
                ..Request::default()
            });
            runtime.schedule(warning,now).unwrap();
            assert_eq!(runtime.store.lock().unwrap().active().iter()
                .map(|n|n.id).collect::<Vec<_>>(),
                if asc{vec![old,warning,unrelated]}
                else{vec![warning,old,unrelated]});
        }
    }

    #[test]
    fn scoped_group_by_controls_grouped_on_notify_binding() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\n[category=warning]\ngroup-by=app-name\n[grouped]\non-notify=dismiss\n"
        ).unwrap());
        let earlier=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"success".into(),
            ..Request::default()
        });
        assert!(matches!(runtime.on_notify_binding(earlier).unwrap(),
            crate::hooks::NotifyBinding::None));
        let warning=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"warning".into(),
            ..Request::default()
        });
        assert!(matches!(runtime.on_notify_binding(warning).unwrap(),
            crate::hooks::NotifyBinding::Dismiss{save_history:true}));
    }

    #[test]
    fn quiet_mode_retains_notifications_and_suppresses_configured_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("unexpected-hook");
        let cfg = format!(
            "[category=success]\non-notify=exec touch '{}'\n[mode=quiet]\non-notify=none\n",
            marker.display()
        );
        let runtime = SharedRuntime::new(Config::parse(&cfg).unwrap());
        *runtime.modes().lock().unwrap() = vec!["quiet".into()];
        let id = runtime.store.lock().unwrap().push(Request {
            category: "success".into(), ..Request::default()
        });
        runtime.schedule(id, Instant::now()).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().len(), 1);
        assert!(!marker.exists(), "quiet mode unexpectedly launched a hook");
    }

    #[test]
    fn mode_changes_affect_new_notification_timeout_rules() {
        let runtime = SharedRuntime::new(
            Config::parse("default-timeout=1000\n[mode=quiet]\ndefault-timeout=0\n").unwrap()
        );
        runtime.modes().lock().unwrap().clear();
        runtime.modes().lock().unwrap().push("quiet".into());
        let id = runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(id, Instant::now()).unwrap();
        assert!(runtime.next_deadline().unwrap().is_none());
    }

    #[test]
    fn restored_note_stays_active_without_a_new_deadline() {
        let runtime = SharedRuntime::new(Config::parse(
            "default-timeout=500\n"
        ).unwrap());
        let first = Instant::now();
        let id = runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(id, first).unwrap();
        assert!(runtime.close(id, CloseReason::Dismissed, true).unwrap());
        assert!(runtime.next_deadline().unwrap().is_none());
        assert_eq!(runtime.store.lock().unwrap().restore(), Some(id));
        runtime.reapply_active_history_policies().unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),None);
        assert!(runtime.expire(first + Duration::from_secs(60)).unwrap().is_empty());
        assert_eq!(runtime.store.lock().unwrap().active()[0].id,id);
    }

    #[test]
    fn grouped_history_style_updates_on_arrival_and_last_member_close() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\n[grouped]\nhistory=0\n"
        ).unwrap());
        let now=Instant::now();
        let first=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        runtime.schedule(first,now).unwrap();
        let second=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        runtime.schedule(second,now).unwrap();
        assert!(runtime.close(second,CloseReason::Dismissed,true).unwrap());
        assert!(runtime.store.lock().unwrap().history().is_empty(),
            "the new group member must use [grouped] history=0");
        assert!(runtime.close(first,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history()[0].id,first,
            "last remaining member becomes ungrouped and regains history=1");
    }

    #[test]
    fn grouped_history_reapplies_after_timer_expiration() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\n[grouped]\nhistory=0\n"
        ).unwrap());
        let now=Instant::now();
        let persistent=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),requested_timeout_ms:0,
            ..Request::default()
        });
        runtime.schedule(persistent,now).unwrap();
        let expiring=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),requested_timeout_ms:100,
            ..Request::default()
        });
        runtime.schedule(expiring,now).unwrap();
        assert_eq!(runtime.expire(now+Duration::from_millis(100)).unwrap(),
            vec![expiring]);
        assert!(runtime.store.lock().unwrap().history().is_empty());
        assert!(runtime.close(persistent,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history()[0].id,persistent);
    }

    #[test]
    fn per_category_history_rules_apply_on_dismissal_and_expiration() {
        let config=Config::parse(
            "default-timeout=100\n[category=transient]\nhistory=0\n"
        ).unwrap();
        let runtime=SharedRuntime::new(config);
        let now=Instant::now();
        let a=runtime.store.lock().unwrap().push(Request {
            category:"transient".into(),..Request::default()
        });
        let b=runtime.store.lock().unwrap().push(Request {
            category:"ordinary".into(),..Request::default()
        });
        runtime.schedule(a,now).unwrap();
        runtime.schedule(b,now).unwrap();
        assert!(runtime.close(a,CloseReason::Dismissed,true).unwrap());
        assert!(runtime.store.lock().unwrap().history().is_empty());
        assert_eq!(runtime.expire(now+Duration::from_millis(100)).unwrap(),vec![b]);
        assert_eq!(runtime.store.lock().unwrap().history()[0].id,b);

        let c=runtime.store.lock().unwrap().push(Request {
            category:"transient".into(),..Request::default()
        });
        runtime.schedule(c,now).unwrap();
        assert_eq!(runtime.expire(now+Duration::from_millis(100)).unwrap(),vec![c]);
        assert_eq!(runtime.store.lock().unwrap().history().len(),1);
    }

    #[test]
    fn replacing_notification_updates_its_history_policy() {
        let runtime=SharedRuntime::new(Config::parse(
            "[category=transient]\nhistory=0\n"
        ).unwrap());
        let now=Instant::now();
        let id=runtime.store.lock().unwrap().push(Request {
            category:"transient".into(),..Request::default()
        });
        runtime.schedule(id,now).unwrap();
        runtime.store.lock().unwrap().push(Request {
            replaces_id:id,category:"durable".into(),..Request::default()
        });
        runtime.schedule(id,now).unwrap();
        assert!(runtime.close(id,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history().len(),1);
    }

    #[test]
    fn reload_replaces_valid_source_atomically_and_preserves_invalid_source() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("mako.conf");
        std::fs::write(&path,
            "default-timeout=500\nbackground-color=#305747\nmax-history=5\n").unwrap();
        let runtime=SharedRuntime::from_path(&path).unwrap();
        let id=runtime.store.lock().unwrap().push(Request::default());
        let now=Instant::now();
        runtime.schedule_without_hook(id,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),Some(now+Duration::from_millis(500)));
        std::fs::write(&path,"default-timeout=broken\n").unwrap();
        assert!(runtime.reload().is_err());
        assert_eq!(runtime.revision(),0);
        assert_eq!(runtime.config().global["background-color"],"#305747");
        std::fs::write(&path,
            "default-timeout=800\nbackground-color=#684148\nmax-history=1\n").unwrap();
        runtime.reload().unwrap();
        assert_eq!(runtime.revision(),1);
        assert_eq!(runtime.config().global["background-color"],"#684148");
        assert_eq!(runtime.next_deadline().unwrap(),Some(now+Duration::from_millis(500)));
        let other=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(other,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),Some(now+Duration::from_millis(500)));
        assert_eq!(runtime.expire(now+Duration::from_millis(500)).unwrap(),vec![id]);
        assert_eq!(runtime.expire(now+Duration::from_millis(800)).unwrap(),vec![other]);
        assert_eq!(runtime.store.lock().unwrap().history().len(),1);
    }

    #[test]
    fn reload_reapplies_existing_history_without_resetting_deadlines() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("config");
        std::fs::write(&path,
            "history=0\ndefault-timeout=1000\n").unwrap();
        let runtime=SharedRuntime::from_path(&path).unwrap();
        let now=Instant::now();
        let first=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(first,now).unwrap();
        let old_deadline=runtime.next_deadline().unwrap();
        assert_eq!(old_deadline,Some(now+Duration::from_millis(1000)));
        std::fs::write(&path,
            "history=1\ndefault-timeout=5000\n").unwrap();
        runtime.reload().unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),old_deadline,
            "style reapplication cannot restart running timers");
        assert!(runtime.close(first,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history()[0].id,first);

        let second=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(second,now).unwrap();
        std::fs::write(&path,"history=0\n").unwrap();
        runtime.reload().unwrap();
        assert!(runtime.close(second,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history().len(),1,
            "a successful reload must update history eligibility");
    }

    #[test]
    fn configured_output_criteria_control_timeout_and_history() {
        let runtime=SharedRuntime::new(Config::parse(
            "output=DP-2\ndefault-timeout=900\n             [output=DP-2]\ndefault-timeout=210\nhistory=0\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let now=Instant::now();
        runtime.schedule_without_hook(id,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(210)));
        assert!(runtime.close(id,CloseReason::Dismissed,true).unwrap());
        assert!(runtime.store.lock().unwrap().history().is_empty());
    }

    #[test]
    fn anchor_selector_controls_timeout_on_non_default_anchor() {
        let runtime=SharedRuntime::new(Config::parse(
            "anchor=bottom-left\ndefault-timeout=1000\n             [anchor=bottom-left]\ndefault-timeout=250\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let now=Instant::now();
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+Duration::from_millis(250)));
    }

    #[test]
    fn sender_timeout_applies_without_ignore_timeout() {
        let runtime = SharedRuntime::new(Config::parse("default-timeout=5000\n").unwrap());
        let now = Instant::now();
        let id = runtime.store.lock().unwrap().push(Request {
            requested_timeout_ms: 800, ..Request::default()
        });
        runtime.schedule(id, now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(), Some(now + Duration::from_millis(800)));
    }
}
