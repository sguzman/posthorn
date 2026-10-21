//! Freedesktop notification D-Bus adapter (not activated by the CLI yet).
//!
//! This intentionally preserves the Mako wire signature. A Wayland renderer,
//! timeouts, and the control interface are still required before serving the
//! session-bus name. NEVER run this beside the installed Mako daemon.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::{interface, object_server::SignalEmitter, zvariant::OwnedValue};

use crate::notification::{Action, CloseReason, NotificationStore, Request, Urgency};
use crate::runtime::SharedRuntime;
use std::time::Instant;

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

/// State shared by the desktop D-Bus API and (eventually) the Wayland runtime.
#[derive(Clone, Debug)]
pub struct FreedesktopNotifications {
    store: Arc<Mutex<NotificationStore>>,
    runtime: Option<SharedRuntime>,
}

impl FreedesktopNotifications {
    pub fn new(store: Arc<Mutex<NotificationStore>>) -> Self {
        Self { store, runtime: None }
    }

    pub fn with_runtime(runtime: SharedRuntime) -> Self {
        Self { store: Arc::clone(&runtime.store), runtime: Some(runtime) }
    }

    pub fn store(&self) -> &Arc<Mutex<NotificationStore>> {
        &self.store
    }

    fn apply_notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        // Freedesktop actions are alternating key/label pairs.
        if actions.len() % 2 != 0 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "notification actions must be key/label pairs".into(),
            ));
        }
        let actions = actions
            .chunks_exact(2)
            .map(|pair| Action {
                key: pair[0].clone(),
                label: pair[1].clone(),
            })
            .collect();

        // A hint that isn't a string is simply not the requested hint.
        let text_hint = |name: &str| {
            hints
                .get(name)
                .and_then(|hint| String::try_from(hint.clone()).ok())
                .unwrap_or_default()
        };

        let mut tag = text_hint("x-dunst-stack-tag");
        if tag.is_empty() {
            tag = text_hint("x-canonical-private-synchronous");
        }
        let urgency = match hints.get("urgency").and_then(|v| u8::try_from(v).ok()) {
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };
        // Historical Freedesktop aliases are still accepted by Mako. The
        // modern image-data hint takes precedence over both image-path and
        // the app_icon Notify argument.
        let raw_hint = ["image-data","image_data","icon_data"].into_iter()
            .find_map(|name| hints.get(name));
        let image_data = raw_hint.and_then(|hint| {
            let data: (i32,i32,i32,bool,i32,i32,Vec<u8>) =
                hint.try_clone().ok()?.try_into().ok()?;
            crate::icons::decode_raw_image(
                data.0,data.1,data.2,data.3,data.4,data.5,&data.6)
        });
        let icon_path = ["image-path","image_path"].into_iter()
            .map(text_hint).find(|s|!s.is_empty())
            .unwrap_or_else(||app_icon.to_string());
        let request = Request {
            replaces_id,
            app_name: app_name.to_string(),
            app_icon: icon_path,
            image_data,
            summary: summary.to_string(),
            body: body.to_string(),
            category: text_hint("category"),
            desktop_entry: text_hint("desktop-entry"),
            tag,
            urgency,
            requested_timeout_ms: expire_timeout,
            actions,
        };
        let mut state = self
            .store
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("notification state lock poisoned".into()))?;
        let id = state.push(request);
        drop(state);
        if let Some(runtime) = &self.runtime {
            runtime.schedule(id, Instant::now())
                .map_err(|e| zbus::fdo::Error::Failed(e.into()))?;
        }
        Ok(id)
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl FreedesktopNotifications {
    #[zbus(name = "GetCapabilities")]
    fn get_capabilities(&self) -> Vec<String> {
        // This represents the intended Mako-compatible API capabilities.
        // The adapter is not registered until its renderer supports them.
        ["body", "body-markup", "actions", "icon-static",
         "x-canonical-private-synchronous", "x-dunst-stack-tag"]
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    #[zbus(name = "GetServerInformation")]
    fn get_server_information(&self) -> (String, String, String, String) {
        // Exact Mako 1.11.0 wire-compatible response; identity changes after
        // MVP require an explicit compatibility decision.
        ("mako".into(), "emersion".into(), "0.0.0".into(), "1.2".into())
    }

    #[zbus(name = "Notify")]
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<u32> {
        let id = self.apply_notify(
            app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout,
        )?;
        crate::signals::invalidate(conn, "Notifications").await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(id)
    }

    #[zbus(name = "CloseNotification")]
    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let closed = if let Some(runtime) = &self.runtime {
            runtime.close(id, CloseReason::ClosedByCall, true)
                .map_err(|e| zbus::fdo::Error::Failed(e.into()))?
        } else {
            let mut state = self.store.lock().map_err(|_| {
                zbus::fdo::Error::Failed("notification state lock poisoned".into())
            })?;
            state.close(id, CloseReason::ClosedByCall, true, true).is_some()
        };
        if closed {
            Self::notification_closed(&emitter, id, CloseReason::ClosedByCall as u32)
                .await
                .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;
            crate::signals::invalidate(emitter.connection(), "Notifications")
                .await.map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> FreedesktopNotifications {
        FreedesktopNotifications::new(Arc::new(Mutex::new(NotificationStore::new(5))))
    }

    #[test]
    fn maps_category_urgency_actions_and_tags() {
        let s = service();
        let hints: HashMap<_, _> = [
            ("category".into(), OwnedValue::try_from(zbus::zvariant::Value::from("success")).unwrap()),
            ("urgency".into(), OwnedValue::from(2u8)),
            ("x-dunst-stack-tag".into(), OwnedValue::try_from(zbus::zvariant::Value::from("job")).unwrap()),
        ].into();
        let id = s.apply_notify("worker", 0, "", "Done", "Completed",
            vec!["default".into(), "Open".into()], hints, 7000).unwrap();
        let store = s.store.lock().unwrap();
        let n = store.active().iter().find(|n| n.id == id).unwrap();
        assert_eq!(n.category, "success");
        assert_eq!(n.tag, "job");
        assert_eq!(n.urgency, Urgency::Critical);
        assert_eq!(n.actions[0].key, "default");
        assert_eq!(n.requested_timeout_ms, 7000);
    }

    #[test]
    fn rejects_odd_action_arrays_without_touching_state() {
        let s = service();
        let result = s.apply_notify("a", 0, "", "oops", "", vec!["orphan".into()],
            HashMap::new(), -1);
        assert!(result.is_err());
        assert!(s.store.lock().unwrap().active().is_empty());
    }

    #[test]
    fn notification_replacement_keeps_identifier() {
        let s = service();
        let id = s.apply_notify("app", 0, "", "old", "", Vec::new(), HashMap::new(), -1).unwrap();
        let new_id = s.apply_notify("app", id, "", "new", "", Vec::new(), HashMap::new(), -1).unwrap();
        assert_eq!(id, new_id);
        assert_eq!(s.store.lock().unwrap().active()[0].summary, "new");
    }
}
