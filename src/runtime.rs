//! Shared notification lifecycle integration for the staged daemon.
//!
//! The D-Bus adapter and timer driver share this state. No database, channels,
//! or UI code is introduced during the strict Mako MVP port.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::config::{Config, Traits};
use crate::notification::{CloseReason, NotificationStore, SortPolicy};
use crate::hooks::NotifyBinding;
use crate::timers::ExpirationScheduler;

#[derive(Clone, Debug)]
pub struct SharedRuntime {
    pub store: Arc<Mutex<NotificationStore>>,
    timers: Arc<Mutex<ExpirationScheduler>>,
    config: Arc<Config>,
    modes: Arc<Mutex<Vec<String>>>,
}

impl SharedRuntime {
    pub fn new(config: Config) -> Self {
        let mut store = NotificationStore::new(config.max_history);
        store.set_sort_policy(SortPolicy::from_config(&config.sort));
        Self {
            store: Arc::new(Mutex::new(store)),
            timers: Arc::new(Mutex::new(ExpirationScheduler::default())),
            config: Arc::new(config),
            modes: Arc::new(Mutex::new(vec!["default".into()])),
        }
    }

    pub fn config(&self) -> &Config { &self.config }

    pub fn modes(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.modes)
    }

    /// Schedule the latest snapshot, never an earlier replaced payload.
    /// Lock order is store -> scheduler everywhere to avoid deadlocks.
    pub fn schedule(&self, id: u32, now: Instant) -> Result<(), &'static str> {
        self.schedule_impl(id, now, true)
    }

    /// Restore already-seen notifications without replaying the on-notify
    /// sound or any configured external hook.
    pub fn schedule_without_hook(&self, id: u32, now: Instant) -> Result<(), &'static str> {
        self.schedule_impl(id, now, false)
    }

    fn schedule_impl(&self, id: u32, now: Instant, run_hook: bool) -> Result<(), &'static str> {
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
                crate::notification::Urgency::Low => "low",
                crate::notification::Urgency::Normal => "normal",
                crate::notification::Urgency::Critical => "critical",
            },
            anchor: "top-right", output: "", modes: &mode_views,
            requested_timeout_ms: note.requested_timeout_ms,
            actionable: !note.actions.is_empty(), group_index: -1, hidden: false,
        };
        let style = self.config.style_for(&traits);
        let default_ms = style.get("default-timeout")
            .and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
        let ignore_sender = style.get("ignore-timeout")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let timeout = note.timeout_ms(default_ms, ignore_sender);
        self.timers.lock().map_err(|_| "expiration scheduler lock poisoned")?
            .set(id, now, timeout);
        // Only configured commands are executable, never message fields.
        // Match Mako's quiet-mode on-notify=none override without suppressing
        // the notification or changing its category color.
        let binding = style.get("on-notify")
            .and_then(|s| NotifyBinding::parse(s).ok());
        drop(modes);
        drop(store);
        if let (true, Some(NotifyBinding::Exec(command))) = (run_hook, binding) {
            if let Err(error) = NotifyBinding::Exec(command).run_exec(id) {
                eprintln!("posthorn on-notify hook failed: {error}");
            }
        }
        // Non-exec notify bindings are not integrated with D-Bus signals yet.
        Ok(())
    }

    pub fn close(&self, id: u32, reason: CloseReason, save_history: bool)
        -> Result<bool, &'static str>
    {
        let mut store = self.store.lock().map_err(|_| "notification store lock poisoned")?;
        let closed = store.close(id, reason, save_history, true).is_some();
        if closed {
            self.timers.lock().map_err(|_| "expiration scheduler lock poisoned")?
                .cancel(id);
        }
        Ok(closed)
    }

    /// Returns IDs actually expired (already-dismissed timer IDs are ignored).
    pub fn expire(&self, now: Instant) -> Result<Vec<u32>, &'static str> {
        let mut store = self.store.lock().map_err(|_| "notification store lock poisoned")?;
        let mut timers = self.timers.lock()
            .map_err(|_| "expiration scheduler lock poisoned")?;
        Ok(timers.expire_active(now, &mut store, true))
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
    fn restored_note_rearms_its_deadline_without_replaying_sound() {
        let runtime = SharedRuntime::new(Config::parse(
            "default-timeout=500\n"
        ).unwrap());
        let first = Instant::now();
        let id = runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(id, first).unwrap();
        assert!(runtime.close(id, CloseReason::Dismissed, true).unwrap());
        assert!(runtime.next_deadline().unwrap().is_none());
        assert_eq!(runtime.store.lock().unwrap().restore(), Some(id));
        runtime.schedule_without_hook(id, first + Duration::from_millis(200)).unwrap();
        assert_eq!(runtime.next_deadline().unwrap(),
            Some(first + Duration::from_millis(700)));
        assert!(runtime.expire(first + Duration::from_millis(499)).unwrap().is_empty());
        assert_eq!(runtime.expire(first + Duration::from_millis(700)).unwrap(),vec![id]);
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
