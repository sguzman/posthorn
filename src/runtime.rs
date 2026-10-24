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
    /// A Notify binding is chosen before regrouping but executed afterward.
    pending_notify_bindings: Arc<Mutex<HashMap<u32,NotifyBinding>>>,
    config: Arc<RwLock<Arc<Config>>>,
    // The compositor's wl_surface.enter output name, not config.output.
    // Unknown until the layer surface is mapped and the compositor replies.
    surface_output: Arc<RwLock<Option<String>>>,
    /// Mako criteria.c retains mako_surface objects keyed by configured
    /// output, layer and anchor, even when their wl_surface is unmapped.
    /// This records only that matching identity. A true multi-surface
    /// renderer with independent entered outputs remains future work.
    known_surface_specs: Arc<Mutex<Vec<(String,String,String)>>>,
    source_path: Option<Arc<PathBuf>>,
    /// Original Mako CLI style args are replayed after each disk reload.
    cli_overrides: Arc<Vec<(String,String)>>,
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
            pending_notify_bindings: Arc::new(Mutex::new(HashMap::new())),
            config: Arc::new(RwLock::new(Arc::new(config))),
            surface_output: Arc::new(RwLock::new(None)),
            known_surface_specs: Arc::new(Mutex::new(Vec::new())),
            source_path: None,
            cli_overrides: Arc::new(Vec::new()),
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

    /// Output selectors in Mako match wl_surface.enter's actual monitor.
    /// Configured target names are only layer-shell placement requests.
    pub fn surface_output(&self)->Option<String>{
        self.surface_output.read()
            .expect("surface output lock poisoned").clone()
    }

    /// Called by the private Wayland presenter after compositor output events.
    /// A changed output invalidates styled frames and group visibility.
    pub fn set_surface_output(&self,name:Option<&str>)->Result<bool,String>{
        let mut current=self.surface_output.write()
            .map_err(|_|"surface output lock poisoned")?;
        if current.as_deref()==name{return Ok(false);}
        *current=name.map(str::to_owned);
        drop(current);
        // Mako render.c rematches criteria when a wl_surface enters/leaves
        // its compositor output. Refresh the associated history policy too,
        // or an output-scoped history=0 could archive a dismissed popup.
        // Do not restart the original notification timeout.
        self.reapply_active_history_policies()?;
        self.revision.fetch_add(1,Ordering::Release);
        Ok(true)
    }

    /// Receive the compositor-observed output for one independently
    /// associated notification. The legacy one-surface presenter continues
    /// to call set_surface_output; this does not yet create a second popup.
    pub fn set_notification_surface_output(
        &self,id:u32,name:Option<&str>
    )->Result<bool,String>{
        let changed=self.store.lock()
            .map_err(|_|"notification store lock poisoned")?
            .set_note_surface_output(id,name)
            .map_err(str::to_owned)?;
        if changed{
            self.reapply_active_history_policies()?;
            self.revision.fetch_add(1,Ordering::Release);
        }
        Ok(changed)
    }

    pub fn from_path(path:&Path)->Result<Self,crate::config::ConfigError>{
        Self::from_path_with_cli_overrides(path,&[])
    }

    /// Mako parses the config file, then applies global CLI overrides in
    /// the original argv order. Keep the arguments for later Reload calls,
    /// which upstream also replays without restarting existing timers.
    pub fn from_path_with_cli_overrides(
        path:&Path,overrides:&[(String,String)]
    )->Result<Self,crate::config::ConfigError>{
        let mut config=Config::from_path(path)?;
        for (key,value) in overrides{
            config.apply_cli_override(key,value)?;
        }
        let mut runtime=Self::new(config);
        runtime.source_path=Some(Arc::new(path.to_path_buf()));
        runtime.cli_overrides=Arc::new(overrides.to_vec());
        Ok(runtime)
    }

    /// Reload only a source-backed instance. Parse before taking the write
    /// lock; invalid or missing files never displace the working config.
    pub fn reload(&self)->Result<(),String>{
        let path=self.source_path.as_ref()
            .ok_or("reload requires a configuration file path")?;
        let mut updated=Config::from_path(path).map_err(|e|e.to_string())?;
        // Upstream reload_config -> parse_config_arguments repeats the
        // post-file getopt pass. Validate before the active-config write
        // lock, so any invalid source remains wholly non-disruptive.
        for (key,value) in self.cli_overrides.iter(){
            updated.apply_cli_override(key,value).map_err(|e|e.to_string())?;
        }
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
        self.reapply_active_grouping()?;
        self.revision.fetch_add(1,Ordering::Release);
        Ok(())
    }

    /// Match Mako reapply_config: clear prior group indices and rerun
    /// all active notifications' group-by rules in current list order.
    /// Existing notification deadlines are not restarted.
    pub fn reapply_active_grouping(&self)->Result<(),String>{
        let mut store=self.store.lock()
            .map_err(|_|"notification store lock poisoned")?;
        let modes=self.modes.lock()
            .map_err(|_|"mode state lock poisoned")?.clone();
        // Mako dbus/mako.c:reapply_config destroys ALL old mako_surface
        // objects and clears every notif->surface before reapplying styles.
        // Rebuild the reusable identity registry in notification-list order.
        self.known_surface_specs.lock()
            .map_err(|_|"surface identity lock poisoned")?.clear();
        store.clear_surface_associations();
        let ids:Vec<u32>=store.active().iter().map(|n|n.id).collect();
        store.clear_recorded_grouping();
        for id in ids {
            let Some(note)=store.active().iter().find(|n|n.id==id).cloned()
                else{continue};
            let style=crate::desktop::style_for_note_index(self,&note,&modes,-1);
            self.associate_selected_surface(&mut store,id,&style)?;
            let spec=crate::grouping::GroupSpec::parse(
                style.get("group-by").map(String::as_str).unwrap_or("none")
            )?;
            store.regroup_new_notification(id,&spec);
        }
        drop(store);
        self.reapply_active_history_policies()
    }

    /// Mako v1.11 criteria.c:apply_each_criteria searches all existing
    /// mako_surfaces for the newly selected (output,layer,anchor) triplet.
    /// A match overrides a direct replacement's old association; without
    /// a match it retains its old surface, or creates a new one if NULL.
    /// This is the identity decision, not multi-layer rendering itself.
    fn associate_selected_surface(
        &self,store:&mut NotificationStore,id:u32,
        style:&crate::config::Options,
    )->Result<(),String>{
        let key=(
            style.get("output").cloned().unwrap_or_default(),
            style.get("layer").cloned().unwrap_or_else(||"top".into()),
            style.get("anchor").cloned().unwrap_or_else(||"top-right".into()),
        );
        let mut known=self.known_surface_specs.lock()
            .map_err(|_|"surface identity lock poisoned")?;
        let has_match=known.contains(&key);
        let has_old=store.active().iter().find(|n|n.id==id)
            .ok_or("notification is not active")?.surface_spec.is_some();
        if has_match{
            // Notifications matching one Mako surface must use ONE output
            // observation. A direct replacement may transfer to that surface.
            let observation=store.active().iter()
                .find(|n|n.surface_spec.as_ref()==Some(&key))
                .map(|n|n.surface_output_override.clone())
                .unwrap_or(Some(None));
            store.bind_surface(id,key,observation);
        }else if !has_old{
            known.push(key.clone());
            store.bind_surface(id,key,None);
        }
        Ok(())
    }

    /// Recalculate active notification history eligibility after Mako-style
    /// mode/config reapplication. This does not reschedule timers or trigger
    /// on-notify bindings, which upstream also leaves alone.
    /// Mako RestoreNotification reinitializes a historical note's style,
    /// applies criteria with a NULL surface, then associates a surface.
    /// Do not treat this as a new Notify: no timer or on-notify hook is armed.
    pub fn associate_restored_surface(&self,id:u32)->Result<(),String>{
        let mut store=self.store.lock()
            .map_err(|_|"notification store lock poisoned")?;
        let note=store.active().iter().find(|n|n.id==id)
            .ok_or("restored notification is not active")?.clone();
        let modes=self.modes.lock()
            .map_err(|_|"mode state lock poisoned")?.clone();
        let mut unassociated=note.clone();
        unassociated.surface_associated=false;
        unassociated.surface_anchor=None;
        let style=crate::desktop::style_for_note_index(
            self,&unassociated,&modes,note.group_index);
        self.associate_selected_surface(&mut store,id,&style)?;
        Ok(())
    }

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
            let index=if note.grouping_applied{note.group_index}
                else{indices.get(&note.id).copied().unwrap_or(-1)};
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
        // reset_notification retains group_index for an explicit ID replacement.
        // Source Mako resolves that index before regrouping its new payload.
        let style=crate::desktop::style_for_note_index(self,&note,&modes,note.group_index);
        let spec=crate::grouping::GroupSpec::parse(
            style.get("group-by").map(String::as_str).unwrap_or("none")
        ).map_err(|_|"invalid group-by style")?;
        self.store.lock().map_err(|_|"notification store lock poisoned")?
            .regroup_new_notification(id,&spec);
        Ok(())
    }

    fn schedule_impl(&self, id: u32, now: Instant, run_hook: bool) -> Result<(), &'static str> {
        let config=self.config();
        let mut store = self.store.lock().map_err(|_| "notification store lock poisoned")?;
        let note = store.active().iter().find(|n| n.id == id)
            .ok_or("notification is not active")?;
        let modes = self.modes.lock().map_err(|_| "mode state lock poisoned")?;
        let mode_views: Vec<&str> = modes.iter().map(String::as_str).collect();
        let observed_output=self.surface_output();
        let traits = Traits {
            app_name: &note.app_name, app_icon: &note.app_icon,
            category: &note.category, desktop_entry: &note.desktop_entry,
            summary: &note.summary, body: &note.body,
            urgency:note.urgency.criteria_label(),
            // Mako has NULL notif->surface during initial Notify criteria.
            anchor: if note.surface_associated {
                note.surface_anchor.as_deref().or_else(||
                    config.global.get("anchor").map(String::as_str)
                ).unwrap_or("top-right")
            }else{""},
            output: if note.surface_associated {
                note.surface_output_override.as_ref()
                    .map(|value|value.as_deref().unwrap_or(""))
                    .unwrap_or(observed_output.as_deref().unwrap_or(""))
            }else{""},
            modes: &mode_views,
            requested_timeout_ms: note.requested_timeout_ms,
            // Mako applies criteria to the preserved group-index on direct ID
            // replacement, before running group_notifications. A newly
            // created/tagged notification starts with group_index = -1.
            actionable: !note.actions.is_empty(), group_index: note.group_index, hidden: false,
        };
        let style = config.style_for(&traits);
        let default_ms = style.get("default-timeout")
            .and_then(|x| crate::config::parse_nonnegative_mako(x)).unwrap_or(0);
        let ignore_sender = style.get("ignore-timeout")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let timeout = note.timeout_ms(default_ms, ignore_sender);
        self.timers.lock().map_err(|_| "expiration scheduler lock poisoned")?
            .set(id, now, timeout);
        let history=style.get("history")
            .is_none_or(|v|v=="1"||v.eq_ignore_ascii_case("true"));
        self.history_policies.lock().map_err(|_| "history policy lock poisoned")?
            .insert(id,history);
        // Mako dbus/xdg.c applies criteria before group_notifications,
        // retaining the chosen notify binding on the style object. It
        // executes the binding after grouping, without reapplying criteria.
        // Capture that decision NOW; group-index may change during regroup.
        let mut pending=self.pending_notify_bindings.lock()
            .map_err(|_|"on-notify binding state lock poisoned")?;
        if run_hook {
            let raw=style.get("on-notify").map(String::as_str).unwrap_or("none");
            pending.insert(id,NotifyBinding::parse(raw)
                .map_err(|_|"invalid on-notify binding")?);
        }else{
            pending.remove(&id);
        }
        // Mako associates the chosen surface after applying initial style.
        // Direct-ID replacement inherits it; fresh/tag replacements do not.
        self.associate_selected_surface(&mut store,id,&style)
            .map_err(|_|"surface identity association failed")?;
        Ok(())
    }

    /// Take the pre-regroup Notify binding after group placement. This
    /// preserves Mako's timing: selected before grouping, executed after.
    /// The D-Bus adapter emits signals only on its async thread.
    pub fn on_notify_binding(&self,id:u32)->Result<NotifyBinding,&'static str>{
        self.pending_notify_bindings.lock()
            .map_err(|_|"on-notify binding state lock poisoned")?
            .remove(&id).ok_or("no staged on-notify binding for ID")
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
            self.pending_notify_bindings.lock()
                .map_err(|_|"on-notify binding state lock poisoned")?
                .remove(&id);
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
            {
                let mut pending=self.pending_notify_bindings.lock()
                    .map_err(|_|"on-notify binding state lock poisoned")?;
                for id in &closed {pending.remove(id);}
            }
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
    fn reload_replays_cli_options_after_disk_file_in_original_order(){
        let dir=tempfile::tempdir().unwrap();
        let file=dir.path().join("mako.conf");
        std::fs::write(&file,"width=215\nmax-history=7\n\
            default-timeout=5000\n[category=warning]\nwidth=340\n").unwrap();
        let overrides=vec![
            ("width".into(),"450".into()),
            ("max-history".into(),"2".into()),
            ("sort".into(),"+priority".into()),
            ("sort".into(),"-priority".into()),
        ];
        let runtime=SharedRuntime::from_path_with_cli_overrides(&file,&overrides).unwrap();
        assert_eq!(runtime.config().global["width"],"450");
        assert_eq!(runtime.config().max_history,2);
        assert_eq!(runtime.config().sort,vec!["-time","-priority"]);
        let now=Instant::now();
        let id=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(id,now).unwrap();
        let deadline=runtime.next_deadline().unwrap();
        std::fs::write(&file,"width=310\nmax-history=9\n\
            default-timeout=100\n[category=warning]\nwidth=500\n").unwrap();
        runtime.reload().unwrap();
        let updated=runtime.config();
        assert_eq!(updated.global["width"],"450",
            "disk file must not override persistent CLI arguments");
        assert_eq!(updated.max_history,2);
        assert_eq!(updated.rules[0].options["width"],"500");
        assert_eq!(runtime.next_deadline().unwrap(),deadline,
            "CLI-aware reload may not reset an active notification timer");
        assert_eq!(runtime.revision(),1);
        std::fs::write(&file,"width=invalid\n").unwrap();
        assert!(runtime.reload().is_err());
        assert_eq!(runtime.config().global["width"],"450");
        assert_eq!(runtime.revision(),1,
            "failed reload must preserve accepted config and revision");
        assert!(SharedRuntime::from_path_with_cli_overrides(
            &file,&[("width".into(),"0".into())]
        ).is_err());
    }

    #[test]
    fn default_timeout_with_leading_space_still_arms_expiration(){
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout= 350\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(id,Instant::now()).unwrap();
        assert!(runtime.next_deadline().unwrap().is_some(),
            "Mako accepts a whitespace-prefixed default-timeout");
    }

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
    fn direct_id_replacement_uses_preserved_group_index_for_initial_timeout() {
        // Mako dbus/xdg.c resets the existing notification in place, keeping
        // its index. apply_each_criteria evaluates that index before the
        // next group_notifications call. A tagged replacement instead
        // constructs a new object with group_index = -1.
        let config=Config::parse(
            "group-by=app-name\ndefault-timeout=0\n\
             [group-index=1]\ndefault-timeout=240\n"
        ).unwrap();
        let runtime=SharedRuntime::new(config);
        let now=Instant::now();
        let old=runtime.store.lock().unwrap().push(Request {
            app_name:"same-app".into(),..Request::default()
        });
        runtime.schedule(old,now).unwrap();
        let newer=runtime.store.lock().unwrap().push(Request {
            app_name:"same-app".into(),..Request::default()
        });
        runtime.schedule(newer,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==old).unwrap().group_index,1);
        assert_eq!(runtime.next_deadline().unwrap(),None);

        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:old,app_name:"same-app".into(),
            ..Request::default()
        }),old);
        let replaced=now+Duration::from_millis(20);
        runtime.schedule(old,replaced).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(replaced+Duration::from_millis(240)),
            "replaced group-index=1 must apply its indexed timeout");

        // Even if regrouping changes the group after replacement, the timer
        // established before regrouping retains the pre-grouping style.
        assert!(runtime.expire(replaced+Duration::from_millis(239))
            .unwrap().is_empty());
        assert_eq!(runtime.expire(replaced+Duration::from_millis(240))
            .unwrap(),vec![old]);
    }

    #[test]
    fn tag_replacement_starts_ungrouped_when_selecting_timeout() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\ndefault-timeout=0\n\
             [group-index=1]\ndefault-timeout=240\n"
        ).unwrap());
        let now=Instant::now();
        let tagged=Request {
            app_name:"tag-app".into(),tag:"job".into(),
            ..Request::default()
        };
        let first=runtime.store.lock().unwrap().push(tagged.clone());
        runtime.schedule(first,now).unwrap();
        let second=runtime.store.lock().unwrap().push(Request {
            app_name:"tag-app".into(),..Request::default()
        });
        runtime.schedule(second,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==first).unwrap().group_index,1);
        assert_eq!(runtime.store.lock().unwrap().push(tagged),first);
        assert_eq!(runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==first).unwrap().group_index,-1);
        runtime.schedule(first,now+Duration::from_millis(10)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),None,
            "new stack-tag object must not inherit the replaced indexed timeout");
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
    fn mode_reapplication_regroups_existing_notifications_without_timers() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\n[mode=quiet]\ngroup-by=app-name\n"
        ).unwrap());
        let now=Instant::now();
        let older=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        runtime.schedule(older,now).unwrap();
        let other=runtime.store.lock().unwrap().push(Request{
            app_name:"other".into(),..Request::default()
        });
        runtime.schedule(other,now).unwrap();
        let newest=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        runtime.schedule(newest,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .map(|n|n.id).collect::<Vec<_>>(),vec![newest,other,older]);
        assert_eq!(crate::grouping::active_group_index(&runtime,newest).unwrap(),-1);
        *runtime.modes.lock().unwrap()=vec!["quiet".into()];
        runtime.reapply_active_grouping().unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .map(|n|n.id).collect::<Vec<_>>(),vec![newest,older,other]);
        assert_eq!(crate::grouping::active_group_index(&runtime,older).unwrap(),1);
        assert!(runtime.next_deadline().unwrap().is_none());
        *runtime.modes.lock().unwrap()=vec!["default".into()];
        runtime.reapply_active_grouping().unwrap();
        assert!(runtime.store.lock().unwrap().active().iter()
            .all(|n|n.group_index==-1));
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
    fn on_notify_binding_uses_pre_regroup_style_and_preserves_direct_replace_index() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\n[category=warning]\ngroup-by=app-name\n[grouped]\non-notify=dismiss\n"
        ).unwrap());
        let now=Instant::now();
        let earlier=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"success".into(),
            ..Request::default()
        });
        runtime.schedule(earlier,now).unwrap();
        assert_eq!(runtime.on_notify_binding(earlier).unwrap(),
            crate::hooks::NotifyBinding::None);
        let warning=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"warning".into(),
            ..Request::default()
        });
        runtime.schedule(warning,now).unwrap();
        assert_eq!(runtime.on_notify_binding(warning).unwrap(),
            crate::hooks::NotifyBinding::None,
            "new arrival was ungrouped while Mako selected on-notify");
        assert_eq!(runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==warning).unwrap().group_index,0);
        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:warning,app_name:"sender".into(),
            category:"warning".into(),..Request::default()
        }),warning);
        runtime.schedule(warning,now).unwrap();
        assert_eq!(runtime.on_notify_binding(warning).unwrap(),
            crate::hooks::NotifyBinding::Dismiss{save_history:true},
            "explicit replacement retains the earlier group-index for binding selection");
        assert!(runtime.on_notify_binding(warning).is_err(),
            "an on-notify hook must be consumed only once");
    }

    #[test]
    fn stack_tag_hook_selection_uses_new_objects_ungrouped_index() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\n[grouped]\non-notify=dismiss\n"
        ).unwrap());
        let now=Instant::now();
        let tagged=Request {
            app_name:"tag-app".into(),tag:"progress".into(),..Request::default()
        };
        let id=runtime.store.lock().unwrap().push(tagged.clone());
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.on_notify_binding(id).unwrap(),NotifyBinding::None);
        let other=runtime.store.lock().unwrap().push(Request {
            app_name:"tag-app".into(),..Request::default()
        });
        runtime.schedule(other,now).unwrap();
        assert_eq!(runtime.on_notify_binding(other).unwrap(),NotifyBinding::None);
        assert_eq!(runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().group_index,1);
        assert_eq!(runtime.store.lock().unwrap().push(tagged),id);
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.on_notify_binding(id).unwrap(),NotifyBinding::None,
            "stack-tag replacement must select its hook before fresh grouping");
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
    fn history_restore_reselects_category_surface_without_new_timeout() {
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout=4000\n[category=warning]\nanchor=bottom-left\n\
             [anchor=bottom-left]\nbackground-color=#416276\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request{
            category:"warning".into(),..Request::default()
        });
        let now=Instant::now();
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().front().unwrap()
            .surface_anchor.as_deref(),Some("bottom-left"));
        assert!(runtime.close(id,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history().front().unwrap()
            .surface_anchor,None);
        assert_eq!(runtime.store.lock().unwrap().restore(),Some(id));
        runtime.associate_restored_surface(id).unwrap();
        runtime.reapply_active_history_policies().unwrap();
        let note=runtime.store.lock().unwrap().active().front().unwrap().clone();
        assert_eq!(note.surface_anchor.as_deref(),Some("bottom-left"));
        let modes=vec!["default".into()];
        assert_eq!(crate::desktop::style_for_note_index(&runtime,&note,&modes,-1)
            .get("background-color").map(String::as_str),Some("#416276"));
        assert!(runtime.next_deadline().unwrap().is_none(),
            "Mako history restore must not schedule a new expiration");
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
    fn replaced_tag_keeps_old_leaders_grouped_history_policy() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\n[category=warning]\ngroup-by=app-name\n[grouped]\nhistory=0\n"
        ).unwrap());
        let now=Instant::now();
        let old=runtime.store.lock().unwrap().push(Request{
            app_name:"app".into(),tag:"same".into(),..Request::default()
        });
        runtime.schedule(old,now).unwrap();
        let leader=runtime.store.lock().unwrap().push(Request{
            app_name:"app".into(),category:"warning".into(),..Request::default()
        });
        runtime.schedule(leader,now).unwrap();
        let tag=runtime.store.lock().unwrap().push(Request{
            app_name:"app".into(),tag:"same".into(),..Request::default()
        });
        assert_eq!(tag,old);
        runtime.schedule(tag,now).unwrap();
        assert_eq!(crate::grouping::active_group_index(&runtime,leader).unwrap(),0);
        assert!(runtime.close(leader,CloseReason::Dismissed,true).unwrap());
        assert!(runtime.store.lock().unwrap().history().is_empty(),
            "stale-but-recorded grouped leader keeps history=0");
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
    fn reload_history_limit_preserves_existing_entries_until_next_addition() {
        // Mako's reload_config plus reapply_config never walks history.
        // Only close_notification prunes after adding a new history entry.
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("config");
        std::fs::write(&path,"max-history=3\n").unwrap();
        let runtime=SharedRuntime::from_path(&path).unwrap();
        let mut old=Vec::new();
        for _ in 0..3 {
            let id=runtime.store.lock().unwrap().push(Request::default());
            assert!(runtime.close(id,CloseReason::Dismissed,true).unwrap());
            old.push(id);
        }
        let ids=|runtime:&SharedRuntime| {
            runtime.store.lock().unwrap().history().iter()
                .map(|n|n.id).collect::<Vec<_>>()
        };
        assert_eq!(ids(&runtime),vec![old[2],old[1],old[0]]);

        std::fs::write(&path,"max-history=1\n").unwrap();
        runtime.reload().unwrap();
        assert_eq!(ids(&runtime),vec![old[2],old[1],old[0]],
            "reducing the limit must not prune stored history during reload");

        let added=runtime.store.lock().unwrap().push(Request::default());
        assert!(runtime.close(added,CloseReason::Dismissed,true).unwrap());
        assert_eq!(ids(&runtime),vec![added],
            "the next saved dismissal must apply the reduced history cap");

        std::fs::write(&path,"max-history=0\n").unwrap();
        runtime.reload().unwrap();
        assert_eq!(ids(&runtime),vec![added],
            "disabling future history must not delete prior entries");
        let ignored=runtime.store.lock().unwrap().push(Request::default());
        assert!(runtime.close(ignored,CloseReason::Dismissed,true).unwrap());
        assert_eq!(ids(&runtime),vec![added],
            "a dismissal at max-history=0 does not delete old history");
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
        let now=Instant::now();
        let before_enter=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(before_enter,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(900)),
            "a configured target alone must not trigger output-scoped timeout");
        assert!(runtime.close(before_enter,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history().len(),1,
            "the unmatched output-scoped history=0 must not suppress history");
        // Even after another surface entered DP-2, a *new* notification
        // starts with NULL notif->surface and cannot match [output] while
        // choosing its initial timeout.
        runtime.set_surface_output(Some("DP-2")).unwrap();
        let after_enter=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(after_enter,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(900)));
        assert!(runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==after_enter).unwrap().surface_associated);
        // A reused Mako surface tuple may be temporarily unmapped after
        // its previous popup was closed. The replacement can match output
        // only after THIS surface receives a new compositor enter event.
        runtime.set_notification_surface_output(after_enter,Some("DP-2")).unwrap();
        // Explicit ID replacement retains the earlier surface association,
        // allowing output selectors to influence its initial timeout.
        assert_eq!(runtime.store.lock().unwrap().push(Request{
            replaces_id:after_enter,..Request::default()
        }),after_enter);
        runtime.schedule_without_hook(after_enter,now+Duration::from_millis(5)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(215)));
        assert!(runtime.close(after_enter,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history().len(),1,
            "output-matched history=0 must not add a history entry");
    }

    #[test]
    fn anchor_selector_requires_preexisting_surface_for_initial_timeout() {
        let runtime=SharedRuntime::new(Config::parse(
            "anchor=bottom-left\ndefault-timeout=1000\n             [anchor=bottom-left]\ndefault-timeout=250\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let now=Instant::now();
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+Duration::from_millis(1000)),
            "new notifications have no surface during initial criteria");
        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:id,..Request::default()
        }),id);
        runtime.schedule(id,now+Duration::from_millis(5)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+Duration::from_millis(255)),
            "direct replacement inherits the existing surface anchor");
    }

    #[test]
    fn direct_replacement_matches_existing_category_selected_surface_anchor() {
        // Mako criteria.c uses notif->surface->anchor, not root anchor.
        // A first category rule can position the surface at bottom-left.
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout=4000\n[category=warning]\nanchor=bottom-left\n\
             [anchor=bottom-left]\ndefault-timeout=1000\n"
        ).unwrap());
        let now=Instant::now();
        let id=runtime.store.lock().unwrap().push(Request {
            category:"warning".into(),..Request::default()
        });
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+Duration::from_millis(4000)));
        let anchor=runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==id).unwrap().surface_anchor.clone();
        assert_eq!(anchor.as_deref(),Some("bottom-left"));

        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:id,category:"normal".into(),..Request::default()
        }),id);
        runtime.schedule(id,now+Duration::from_millis(20)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+Duration::from_millis(1020)),
            "the old attached bottom-left surface must select [anchor]");
        let note=runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==id).unwrap().clone();
        assert_eq!(note.surface_anchor.as_deref(),Some("bottom-left"));

        let fresh=runtime.store.lock().unwrap().push(Request {
            category:"normal".into(),..Request::default()
        });
        runtime.schedule(fresh,now+Duration::from_millis(40)).unwrap();
        // Confirm fresh normal notification does NOT inherit bottom-left.
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==fresh).unwrap().surface_anchor.as_deref(),
            Some("top-right"));
    }

    #[test]
    fn existing_matching_surface_reassigns_direct_replacement_but_mismatched_tuple_does_not() {
        // Upstream searches (configured_output, layer, anchor), not just
        // the desired anchor. All surfaces persist in its state->surfaces.
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout=4000\n[category=left]\nanchor=bottom-left\n\
             [category=right]\nanchor=top-right\noutput=DP-2\nlayer=overlay\n\
             [anchor=top-right]\ndefault-timeout=1000\n"
        ).unwrap());
        let now=Instant::now();
        let left=runtime.store.lock().unwrap().push(Request{
            category:"left".into(),..Request::default()
        });
        runtime.schedule_without_hook(left,now).unwrap();
        let mismatched=runtime.store.lock().unwrap().push(Request{
            category:"right".into(),..Request::default()
        });
        runtime.schedule_without_hook(mismatched,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==mismatched).unwrap().surface_anchor.as_deref(),
            Some("top-right"));
        // The right anchor surface has a different output/layer; a
        // normal replacement must still keep the bottom-left surface.
        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:left,..Request::default()
        }),left);
        runtime.schedule_without_hook(left,now+std::time::Duration::from_millis(10)).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==left).unwrap().surface_anchor.as_deref(),
            Some("bottom-left"));
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(4000)));
        // Now a root top-right surface is actually present.
        let root=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(root,now+std::time::Duration::from_millis(20)).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==root).unwrap().surface_anchor.as_deref(),
            Some("top-right"));
        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:left,..Request::default()
        }),left);
        runtime.schedule_without_hook(left,now+std::time::Duration::from_millis(30)).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==left).unwrap().surface_anchor.as_deref(),
            Some("top-right"),"an existing matching surface wins over old association");
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(4000)),
            "surface relocation happens after timeout selection, not before");
    }

    #[test]
    fn output_enter_and_leave_rematch_history_without_restarting_timeout() {
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout=5000\nhistory=1\n[output=DP-1]\nhistory=0\n"
        ).unwrap());
        let now=Instant::now();
        let first=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(first,now).unwrap();
        let deadline=runtime.next_deadline().unwrap();
        assert_eq!(deadline,Some(now+std::time::Duration::from_millis(5000)));
        runtime.set_surface_output(Some("DP-1")).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),deadline);
        assert!(runtime.close(first,CloseReason::Dismissed,true).unwrap());
        assert!(runtime.store.lock().unwrap().history().is_empty(),
            "output-entered history=0 must suppress archive");

        let second=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(second,now+std::time::Duration::from_millis(20)).unwrap();
        let second_deadline=runtime.next_deadline().unwrap();
        runtime.set_surface_output(None).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),second_deadline);
        assert!(runtime.close(second,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history().len(),1,
            "an unmapped surface must revert to root history policy");
    }

    #[test]
    fn independent_notification_outputs_do_not_borrow_single_presenter_monitor() {
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout=0\n[output=DP-1]\nbackground-color=#42617B\n\
             [output=DP-2]\nbackground-color=#8C642D\ndefault-timeout=200\n\
             [category=independent]\nanchor=bottom-left\n"
        ).unwrap());
        runtime.set_surface_output(Some("DP-1")).unwrap();
        let first=runtime.store.lock().unwrap().push(Request::default());
        let second=runtime.store.lock().unwrap().push(Request {
            category:"independent".into(),..Request::default()
        });
        let now=Instant::now();
        runtime.schedule_without_hook(first,now).unwrap();
        runtime.schedule_without_hook(second,now).unwrap();
        assert!(runtime.set_notification_surface_output(second,Some("DP-2")).unwrap());
        assert!(!runtime.set_notification_surface_output(second,Some("DP-2")).unwrap());
        let colors=||{
            let store=runtime.store.lock().unwrap();
            let modes=vec!["default".into()];
            store.active().iter().map(|note|{
                crate::desktop::style_for_note_index(
                    &runtime,note,&modes,note.group_index)
                .get("background-color").cloned()
            }).collect::<Vec<_>>()
        };
        // Store order is newest-first under Mako's default descending time.
        assert_eq!(colors(),vec![Some("#8C642D".into()),Some("#42617B".into())]);
        runtime.set_surface_output(Some("DP-3")).unwrap();
        let colors_after=colors();
        assert_eq!(colors_after[0].as_deref(),Some("#8C642D"));
        assert_ne!(colors_after[1].as_deref(),Some("#42617B"));
        assert_eq!(runtime.store.lock().unwrap().push(Request {
            replaces_id:second,category:"independent".into(),
            ..Request::default()
        }),second);
        runtime.schedule_without_hook(second,now+std::time::Duration::from_millis(10)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(210)),
            "direct replacement reuses its own DP-2 output for timeout criteria");
        runtime.set_notification_surface_output(second,None).unwrap();
        assert_ne!(colors()[0].as_deref(),Some("#8C642D"),
            "explicitly unmapped surface cannot borrow DP-3");
    }

    #[test]
    fn notifications_sharing_one_surface_share_compositor_output_and_replacement() {
        let runtime=SharedRuntime::new(Config::parse(
            "[output=DP-1]\nbackground-color=#42617B\n\
             [output=DP-2]\nbackground-color=#8C642D\n"
        ).unwrap());
        let now=Instant::now();
        let first=runtime.store.lock().unwrap().push(Request::default());
        let second=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(first,now).unwrap();
        runtime.schedule_without_hook(second,now).unwrap();
        let ids=runtime.store.lock().unwrap().active().iter()
            .map(|n|n.surface_spec.clone()).collect::<Vec<_>>();
        assert_eq!(ids[0],ids[1],"same tuple is one Mako surface");
        assert!(runtime.set_notification_surface_output(first,Some("DP-1")).unwrap());
        assert!(!runtime.set_notification_surface_output(second,Some("DP-1")).unwrap());
        assert!(runtime.store.lock().unwrap().active().iter()
            .all(|n|n.surface_output_override==Some(Some("DP-1".into()))));
        assert!(runtime.set_notification_surface_output(second,Some("DP-2")).unwrap());
        assert!(runtime.store.lock().unwrap().active().iter()
            .all(|n|n.surface_output_override==Some(Some("DP-2".into()))));
        assert_eq!(runtime.store.lock().unwrap().push(Request{
            replaces_id:first,..Request::default()
        }),first);
        runtime.schedule_without_hook(first,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==first).unwrap().surface_output_override,
            Some(Some("DP-2".into())));
    }

    #[test]
    fn mode_reapplication_discards_surface_registry_and_reselects_anchor() {
        let runtime=SharedRuntime::new(Config::parse(
            "default-timeout=3000\n[mode=quiet]\nanchor=bottom-left\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let now=Instant::now();
        runtime.schedule_without_hook(id,now).unwrap();
        let anchor=||runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==id).unwrap().surface_anchor.clone();
        assert_eq!(anchor().as_deref(),Some("top-right"));
        assert_eq!(runtime.known_surface_specs.lock().unwrap().len(),1);
        *runtime.modes.lock().unwrap()=vec!["quiet".into()];
        runtime.reapply_active_grouping().unwrap();
        assert_eq!(anchor().as_deref(),Some("bottom-left"));
        assert_eq!(runtime.known_surface_specs.lock().unwrap().len(),1,
            "Mako destroys the old top-right surface on mode reapplication");
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(3000)));
        *runtime.modes.lock().unwrap()=vec!["default".into()];
        runtime.reapply_active_grouping().unwrap();
        assert_eq!(anchor().as_deref(),Some("top-right"));
        assert_eq!(runtime.known_surface_specs.lock().unwrap().len(),1);
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+std::time::Duration::from_millis(3000)),
            "mode transitions preserve the original expiration deadline");
    }

    #[test]
    fn tag_replacement_does_not_inherit_surface_scoped_timeout() {
        let runtime=SharedRuntime::new(Config::parse(
            "anchor=bottom-left\ndefault-timeout=1000\n             [anchor=bottom-left]\ndefault-timeout=250\n"
        ).unwrap());
        let now=Instant::now();
        let tagged=Request {
            app_name:"app".into(),tag:"progress".into(),..Request::default()
        };
        let id=runtime.store.lock().unwrap().push(tagged.clone());
        runtime.schedule(id,now).unwrap();
        assert_eq!(runtime.store.lock().unwrap().push(tagged),id);
        assert!(!runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().surface_associated);
        runtime.schedule(id,now+Duration::from_millis(5)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(now+Duration::from_millis(1005)),
            "a new stack-tag object cannot use its predecessor's surface");
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
