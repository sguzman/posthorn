//! Staged Mako control-interface port. Not registered on the session bus yet.
//! Keep this separate from the Freedesktop interface because Mako exposes both.
//!
//! This module deliberately returns explicit errors for operations whose runtime
//! semantics (group dismissal or reload) are not implemented. The MVP gate is
//! closed until the full interface and renderer are integrated.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::{interface, zvariant::{OwnedValue, Value}, Connection};

use crate::dbus::{BUS_NAME, OBJECT_PATH};
use crate::notification::{CloseReason, Notification, NotificationStore, Urgency};
use crate::runtime::SharedRuntime;

pub const CONTROL_INTERFACE: &str = "fr.emersion.Mako";
pub const CONTROL_PATH: &str = "/fr/emersion/Mako";

#[derive(Clone, Debug)]
pub struct MakoControl {
    store: Arc<Mutex<NotificationStore>>,
    modes: Arc<Mutex<Vec<String>>>,
}

impl MakoControl {
    pub fn new(store: Arc<Mutex<NotificationStore>>) -> Self {
        Self {
            store,
            modes: Arc::new(Mutex::new(vec!["default".into()])),
        }
    }

    /// Share active modes with the same runtime used by the D-Bus receiver.
    pub fn with_runtime(runtime: SharedRuntime) -> Self {
        Self { store: Arc::clone(&runtime.store), modes: runtime.modes() }
    }

    fn snapshot(&self, history: bool) -> zbus::fdo::Result<Vec<HashMap<String, OwnedValue>>> {
        let store = self.store.lock().map_err(|_| internal_error())?;
        let notifications = if history { store.history() } else { store.active() };
        notifications.iter().map(wire_notification).collect()
    }

    fn dismiss_impl(&self, options: &HashMap<String, OwnedValue>) -> zbus::fdo::Result<Vec<u32>> {
        let all = options.get("all").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        let group = options.get("group").and_then(|v| bool::try_from(v).ok()).unwrap_or(false);
        let history = options.get("history").and_then(|v| bool::try_from(v).ok()).unwrap_or(true);
        let id = options.get("id").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);

        if (all && group) || (id != 0 && (all || group)) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "invalid combination of all/group/id".into(),
            ));
        }
        if group {
            // Never silently dismiss only one notification instead of a group.
            return Err(zbus::fdo::Error::NotSupported(
                "group dismissal awaits the Mako group-by port".into(),
            ));
        }
        let mut store = self.store.lock().map_err(|_| internal_error())?;
        let ids = if all {
            store.active().iter().map(|n| n.id).collect::<Vec<_>>()
        } else if id == 0 {
            store.active().front().map(|n| vec![n.id]).unwrap_or_default()
        } else if store.active().iter().any(|n| n.id == id) {
            vec![id]
        } else {
            Vec::new()
        };
        for notification_id in &ids {
            store.close(*notification_id, CloseReason::Dismissed, history, true);
        }
        Ok(ids)
    }

    fn set_modes_impl(&self, modes: Vec<String>) -> zbus::fdo::Result<()> {
        let mut active = self.modes.lock().map_err(|_| internal_error())?;
        *active = modes;
        Ok(())
    }

    fn find_action(&self, id: u32, key: &str) -> zbus::fdo::Result<Option<u32>> {
        let store = self.store.lock().map_err(|_| internal_error())?;
        let note = if id == 0 {
            store.active().front()
        } else {
            store.active().iter().find(|n| n.id == id)
        };
        Ok(note
            .filter(|n| n.actions.iter().any(|a| a.key == key))
            .map(|n| n.id))
    }
}

fn internal_error() -> zbus::fdo::Error {
    zbus::fdo::Error::Failed("notification state lock poisoned".into())
}

fn string_value(value: &str) -> zbus::fdo::Result<OwnedValue> {
    OwnedValue::try_from(Value::from(value))
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
}

fn wire_notification(note: &Notification) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
    let mut obj = HashMap::new();
    for (key, value) in [
        ("app-name", note.app_name.as_str()),
        ("app-icon", note.app_icon.as_str()),
        ("category", note.category.as_str()),
        ("desktop-entry", note.desktop_entry.as_str()),
        ("summary", note.summary.as_str()),
        ("body", note.body.as_str()),
    ] {
        obj.insert(key.into(), string_value(value)?);
    }
    obj.insert("id".into(), OwnedValue::from(note.id));
    obj.insert("urgency".into(), OwnedValue::from(match note.urgency {
        Urgency::Low => 0u8,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    }));
    let actions: HashMap<String, String> = note.actions.iter()
        .map(|action| (action.key.clone(), action.label.clone())).collect();
    obj.insert("actions".into(), OwnedValue::from(actions));
    Ok(obj)
}

/// Standard notification signals originate on the Freedesktop object path,
/// even when the action came through the Mako-specific control API.
async fn emit_notification_signal(
    conn: &Connection,
    name: &str,
    id: u32,
    extra: impl serde::Serialize + zbus::zvariant::DynamicType + zbus::zvariant::Type,
) -> zbus::fdo::Result<()> {
    conn.emit_signal(
        None::<&str>, OBJECT_PATH, BUS_NAME, name, &(id, extra),
    ).await.map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
}

#[interface(name = "fr.emersion.Mako")]
impl MakoControl {
    #[zbus(name = "DismissNotifications")]
    async fn dismiss_notifications(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        let ids = self.dismiss_impl(&options)?;
        for id in ids {
            emit_notification_signal(conn, "NotificationClosed", id,
                CloseReason::Dismissed as u32).await?;
        }
        Ok(())
    }

    #[zbus(name = "InvokeAction")]
    async fn invoke_action(
        &self,
        id: u32,
        key: &str,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        if let Some(id) = self.find_action(id, key)? {
            emit_notification_signal(conn, "ActionInvoked", id, key).await?;
        }
        Ok(())
    }

    #[zbus(name = "RestoreNotification")]
    fn restore_notification(&self) -> zbus::fdo::Result<()> {
        self.store.lock().map_err(|_| internal_error())?.restore();
        Ok(())
    }

    #[zbus(name = "ListNotifications")]
    fn list_notifications(&self) -> zbus::fdo::Result<Vec<HashMap<String, OwnedValue>>> {
        self.snapshot(false)
    }

    #[zbus(name = "ListHistory")]
    fn list_history(&self) -> zbus::fdo::Result<Vec<HashMap<String, OwnedValue>>> {
        self.snapshot(true)
    }

    #[zbus(name = "SetMode")]
    fn set_mode(&self, mode: &str) -> zbus::fdo::Result<()> {
        self.set_modes_impl(vec![mode.into()])
    }

    #[zbus(name = "SetModes")]
    fn set_modes(&self, modes: Vec<String>) -> zbus::fdo::Result<()> {
        self.set_modes_impl(modes)
    }

    #[zbus(name = "ListModes")]
    fn list_modes(&self) -> zbus::fdo::Result<Vec<String>> {
        Ok(self.modes.lock().map_err(|_| internal_error())?.clone())
    }

    #[zbus(property, name = "Modes")]
    fn modes(&self) -> zbus::fdo::Result<Vec<String>> {
        self.list_modes()
    }

    #[zbus(property, name = "Notifications")]
    fn notifications(&self) -> zbus::fdo::Result<Vec<HashMap<String, OwnedValue>>> {
        self.snapshot(false)
    }

    #[zbus(name = "Reload")]
    fn reload(&self) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "reload requires a live config path and surface integration".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::{Request, Action};

    fn fixture() -> MakoControl {
        MakoControl::new(Arc::new(Mutex::new(NotificationStore::new(5))))
    }

    #[test]
    fn list_has_mako_named_fields_and_correct_types() {
        let control = fixture();
        control.store.lock().unwrap().push(Request {
            app_name: "sender".into(), category: "success".into(),
            actions: vec![Action{key:"default".into(),label:"Open".into()}],
            ..Request::default()
        });
        let list = control.snapshot(false).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(u8::try_from(list[0].get("urgency").unwrap()).unwrap(), 1);
        assert_eq!(u32::try_from(list[0].get("id").unwrap()).unwrap(), 1);
        assert!(list[0].contains_key("actions"));
    }

    #[test]
    fn dismiss_honors_history_flag_and_all() {
        let control = fixture();
        for _ in 0..3 {
            control.store.lock().unwrap().push(Request::default());
        }
        let opts = [("all".into(), OwnedValue::from(true)),
            ("history".into(), OwnedValue::from(false))].into();
        assert_eq!(control.dismiss_impl(&opts).unwrap().len(), 3);
        assert!(control.store.lock().unwrap().active().is_empty());
        assert!(control.store.lock().unwrap().history().is_empty());
    }

    #[test]
    fn invalid_group_args_cannot_dismiss_anything() {
        let control = fixture();
        control.store.lock().unwrap().push(Request::default());
        let opts = [("all".into(), OwnedValue::from(true)),
                    ("group".into(), OwnedValue::from(true))].into();
        assert!(control.dismiss_impl(&opts).is_err());
        assert_eq!(control.store.lock().unwrap().active().len(), 1);
    }

    #[test]
    fn modes_can_be_set_without_implicitly_adding_default() {
        let control = fixture();
        control.set_modes_impl(vec!["quiet".into(), "custom".into()]).unwrap();
        assert_eq!(control.list_modes().unwrap(), vec!["quiet", "custom"]);
    }
}
