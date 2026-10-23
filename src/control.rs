//! Mako control-interface implementation on Posthorn's isolated development
//! session-bus name. The public Mako method and property signatures are retained.
//! Group dismissal, modes, and transactional reload are implemented. The
//! Notifications property still loses duplicate action keys during Value
//! conversion; production cutover remains an explicit MVP acceptance gate.

use std::collections::{HashMap,HashSet};
use std::sync::{Arc, Mutex};

use serde::ser::{Serialize, SerializeMap, Serializer};
use zbus::{interface, zvariant::{Dict, OwnedValue, SerializeValue, Signature, Value}, Connection};

use crate::dbus::{BUS_NAME, OBJECT_PATH};
use crate::notification::{CloseReason, Notification, NotificationStore};
use crate::runtime::SharedRuntime;

pub const CONTROL_INTERFACE: &str = "fr.emersion.Mako";
pub const CONTROL_PATH: &str = "/fr/emersion/Mako";
    
/// sd-bus emits the action a{ss} dictionary in linked-list order, including
/// repeated keys. zvariant::Dict is backed by a BTreeMap and cannot represent
/// either property; serialize the dictionary entries directly instead.
#[derive(Clone,Debug,zbus::zvariant::Type)]
#[zvariant(signature = "a{ss}")]
struct OrderedActionDict(Vec<crate::notification::Action>);

impl Serialize for OrderedActionDict {
    fn serialize<S:Serializer>(&self, serializer:S)->Result<S::Ok,S::Error>{
        let mut map=serializer.serialize_map(Some(self.0.len()))?;
        for action in &self.0{
            map.serialize_entry(&action.key,&action.label)?;
        }
        map.end()
    }
}

/// Public Mako control records remain a{sv}. Only the nested actions variant
/// gets custom ordered serialization. The legacy HashMap snapshot stays for
/// internal callers; it is not an exact action-order wire representation.
#[derive(Clone,Debug,zbus::zvariant::Type)]
#[zvariant(signature = "a{sv}")]
struct OrderedNotificationWire {
    fields: HashMap<String,OwnedValue>,
    actions: OrderedActionDict,
}

impl Serialize for OrderedNotificationWire {
    fn serialize<S:Serializer>(&self,serializer:S)->Result<S::Ok,S::Error>{
        // The ordered actions variant is an additional a{sv} entry. Fields
        // deliberately exclude it so no lossy temporary Dict is constructed.
        let mut map=serializer.serialize_map(Some(self.fields.len()+1))?;
        // Mako 1.11 dbus/mako.c emits these entries in this exact order.
        // Iterating HashMap would randomize the on-wire a{sv} field sequence.
        for key in ["app-name","app-icon","category","desktop-entry",
                    "summary","body","id","urgency"] {
            if let Some(value)=self.fields.get(key){
                map.serialize_entry(key,value)?;
            }
        }
        map.serialize_entry("actions",&SerializeValue(&self.actions))?;
        map.end()
    }
}


/// Upstream Mako v1.11 uses a named control-interface error for a rejected
/// reload, not org.freedesktop.DBus.Error.Failed. A failure must leave the
/// currently installed configuration intact.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "fr.emersion.Mako")]
pub enum ReloadError {
    #[zbus(error)]
    ZBus(zbus::Error),
    InvalidConfig(String),
}

#[derive(Clone, Debug)]
pub struct MakoControl {
    store: Arc<Mutex<NotificationStore>>,
    modes: Arc<Mutex<Vec<String>>>,
    runtime: Option<SharedRuntime>,
}

impl MakoControl {
    pub fn new(store: Arc<Mutex<NotificationStore>>) -> Self {
        Self {
            store,
            modes: Arc::new(Mutex::new(vec!["default".into()])),
            runtime: None,
        }
    }

    /// Share active modes with the same runtime used by the D-Bus receiver.
    pub fn with_runtime(runtime: SharedRuntime) -> Self {
        Self { store: Arc::clone(&runtime.store), modes: runtime.modes(), runtime: Some(runtime) }
    }

    fn snapshot(&self, history: bool) -> zbus::fdo::Result<Vec<HashMap<String, OwnedValue>>> {
        let store = self.store.lock().map_err(|_| internal_error())?;
        let notifications = if history { store.history() } else { store.active() };
        notifications.iter().map(wire_notification).collect()
    }

    fn ordered_snapshot(&self,history:bool)->zbus::fdo::Result<Vec<OrderedNotificationWire>>{
        let store=self.store.lock().map_err(|_|internal_error())?;
        let notifications=if history {store.history()}else{store.active()};
        notifications.iter().map(|note|Ok(OrderedNotificationWire{
            fields:wire_notification_fields(note)?,
            actions:OrderedActionDict(note.actions.clone()),
        })).collect()
    }

    fn dismiss_impl(&self, options: &HashMap<String, OwnedValue>) -> zbus::fdo::Result<Vec<u32>> {
        // Mako's sd_bus_message_read("v", expected_type) fails for a
        // present field of the wrong type. Never silently fall back to id=0:
        // that could dismiss the first unrelated notification.
        let boolean = |key:&str,default:bool| -> zbus::fdo::Result<bool> {
            options.get(key).map(|value|bool::try_from(value).map_err(|_| {
                zbus::fdo::Error::InvalidArgs(format!(
                    "{key} must be a D-Bus boolean"))
            })).unwrap_or(Ok(default))
        };
        let all = boolean("all",false)?;
        let group = boolean("group",false)?;
        let history = boolean("history",true)?;
        let id = options.get("id").map(|value|u32::try_from(value).map_err(|_| {
            zbus::fdo::Error::InvalidArgs("id must be a D-Bus uint32".into())
        })).unwrap_or(Ok(0))?;

        if (all && group) || (id != 0 && (all || group)) {
            return Err(zbus::fdo::Error::InvalidArgs(
                "invalid combination of all/group/id".into(),
            ));
        }
        let ids=if group {
            let first=self.store.lock().map_err(|_|internal_error())?
                .active().front().map(|n|n.id);
            match (first,self.runtime.as_ref()) {
                (None,_)=>Vec::new(),
                (Some(id),Some(runtime))=>
                    crate::grouping::active_group_ids(runtime,id)
                        .map_err(zbus::fdo::Error::NotSupported)?,
                (Some(id),None)=>vec![id],
            }
        }else{
            let store=self.store.lock().map_err(|_|internal_error())?;
            if all {store.active().iter().map(|n|n.id).collect::<Vec<_>>()}
            else if id==0 {store.active().front().map(|n|vec![n.id]).unwrap_or_default()}
            else if store.active().iter().any(|n|n.id==id){vec![id]}
            else{Vec::new()}
        };
        for notification_id in &ids {
            if let Some(runtime) = &self.runtime {
                runtime.close(*notification_id, CloseReason::Dismissed, history)
                    .map_err(|e| zbus::fdo::Error::Failed(e.into()))?;
            } else {
                self.store.lock().map_err(|_| internal_error())?
                    .close(*notification_id, CloseReason::Dismissed, history, true);
            }
        }
        Ok(ids)
    }

    fn set_modes_impl(&self, modes: Vec<String>) -> zbus::fdo::Result<()> {
        // Mako's set_modes drops duplicates but preserves the first
        // occurrence and allows an explicitly empty mode list.
        let mut seen=HashSet::new();
        let unique=modes.into_iter().filter(|m|seen.insert(m.clone()))
            .collect();
        let mut active=self.modes.lock().map_err(|_|internal_error())?;
        *active=unique;
        drop(active);
        if let Some(runtime)=&self.runtime {
            runtime.reapply_active_grouping()
                .map_err(zbus::fdo::Error::Failed)?;
        }
        Ok(())
    }

    fn find_action(&self, id: u32, key: &str) -> zbus::fdo::Result<Option<u32>> {
        let store = self.store.lock().map_err(|_| internal_error())?;
        let note = if id == 0 {
            store.active().front()
        } else {
            store.active().iter().find(|n| n.id == id)
        }.cloned();
        drop(store);
        let Some(note)=note else {return Ok(None)};
        // Mako scans the retained action list but its
        // notify_action_invoked() suppresses signals for actions=0 styles.
        if !note.actions.iter().any(|a|a.key==key){return Ok(None);}
        if let Some(runtime)=&self.runtime{
            if !crate::input::can_invoke_action(runtime,note.id,key)
                .map_err(|e|zbus::fdo::Error::Failed(e.to_string()))?{
                return Ok(None);
            }
        }
        Ok(Some(note.id))
    }
}

fn internal_error() -> zbus::fdo::Error {
    zbus::fdo::Error::Failed("notification state lock poisoned".into())
}

fn string_value(value: &str) -> zbus::fdo::Result<OwnedValue> {
    OwnedValue::try_from(Value::from(value))
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
}

fn wire_notification_fields(note: &Notification) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
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
    // Mako serializes the stored signed enum as a D-Bus byte (y),
    // truncating values outside the normal -1/0/1/2 range.
    obj.insert("urgency".into(),OwnedValue::from(note.urgency.mako_value() as u8));
    Ok(obj)
}

fn wire_notification(note:&Notification)->zbus::fdo::Result<HashMap<String,OwnedValue>>{
    let mut obj=wire_notification_fields(note)?;
    // The Notifications property still uses zvariant::Dict, whose BTreeMap
    // collapses duplicate keys and sorts entries. ListNotifications and
    // ListHistory instead serialize the original ordered action list directly.
    let mut actions=Dict::new(&Signature::Str,&Signature::Str);
    for action in &note.actions {
        actions.append(
            Value::from(action.key.as_str()),
            Value::from(action.label.as_str()),
        ).map_err(|e|zbus::fdo::Error::Failed(e.to_string()))?;
    }
    obj.insert("actions".into(),Value::Dict(actions).try_to_owned()
        .map_err(|e|zbus::fdo::Error::Failed(e.to_string()))?);
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
        if !ids.is_empty() {
            for id in ids {
                emit_notification_signal(conn, "NotificationClosed", id,
                    CloseReason::Dismissed as u32).await?;
            }
            crate::signals::invalidate(conn, "Notifications").await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
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
    async fn restore_notification(
        &self, #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        let restored = self.store.lock().map_err(|_| internal_error())?.restore();
        if let (Some(runtime), Some(_id)) = (&self.runtime, restored) {
            // Upstream Mako inserts a restored notification directly into
            // the active list: it never creates another expiration timer.
            // Reapply current history style without replaying notification
            // hooks or scheduling a replacement timeout.
            runtime.reapply_active_history_policies()
                .map_err(zbus::fdo::Error::Failed)?;
        }
        if restored.is_some() {
            crate::signals::invalidate(conn, "Notifications").await
                .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }
        Ok(())
    }

    #[zbus(name = "ListNotifications")]
    fn list_notifications(&self) -> zbus::fdo::Result<Vec<OrderedNotificationWire>> {
        self.ordered_snapshot(false)
    }

    #[zbus(name = "ListHistory")]
    fn list_history(&self) -> zbus::fdo::Result<Vec<OrderedNotificationWire>> {
        self.ordered_snapshot(true)
    }

    #[zbus(name = "SetMode")]
    async fn set_mode(
        &self, mode: &str, #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        self.set_modes_impl(vec![mode.into()])?;
        crate::signals::invalidate(conn, "Modes").await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    #[zbus(name = "SetModes")]
    async fn set_modes(
        &self, modes: Vec<String>, #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        self.set_modes_impl(modes)?;
        crate::signals::invalidate(conn, "Modes").await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
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
        // zbus's #[interface(property)] requires Into<Value>, which
        // materializes the nested dictionary and collapses duplicate keys.
        // Custom ordered serialization works for the two read methods;
        // property fidelity remains an explicit compatibility gap.
        self.snapshot(false)
    }

    #[zbus(name = "Reload")]
    async fn reload(
        &self, #[zbus(connection)] conn:&Connection
    ) -> Result<(),ReloadError> {
        let runtime=self.runtime.as_ref().ok_or_else(||
            ReloadError::InvalidConfig("Unable to parse configuration file".into())
        )?;
        runtime.reload().map_err(|_|
            ReloadError::InvalidConfig("Unable to parse configuration file".into())
        )?;
        crate::signals::invalidate(conn,"Notifications").await?;
        Ok(())
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
        assert_eq!(u8::try_from(list[0].get("urgency").unwrap()).unwrap(), 255);
        assert_eq!(u32::try_from(list[0].get("id").unwrap()).unwrap(), 1);
        assert!(list[0].contains_key("actions"));
    }


    #[test]
    fn out_of_range_mako_urgency_preserves_wire_byte(){
        let control=fixture();
        control.store.lock().unwrap().push(Request{
            urgency:crate::notification::Urgency::Other(-2),
            ..Request::default()
        });
        let record=control.snapshot(false).unwrap();
        assert_eq!(u8::try_from(record[0].get("urgency").unwrap()).unwrap(),254);
    }

    #[test]
    fn notification_record_fields_follow_mako_fixed_dbus_entry_sequence(){
        use zbus::zvariant::{serialized::Context,LE};
        let control=fixture();
        control.store.lock().unwrap().push(Request{
            app_name:"name-fixture".into(),
            app_icon:"icon-fixture".into(),
            category:"category-fixture".into(),
            desktop_entry:"desktop-fixture".into(),
            summary:"summary-fixture".into(),
            body:"body-fixture".into(),
            ..Request::default()
        });
        let notes=control.ordered_snapshot(false).unwrap();
        let wire=zbus::zvariant::to_bytes(Context::new_dbus(LE,0),&notes).unwrap();
        let bytes=wire.bytes();
        let mut previous=0;
        for (index,key) in ["app-name","app-icon","category","desktop-entry",
                          "summary","body","id","urgency","actions"]
            .iter().enumerate(){
            let mut needle=key.as_bytes().to_vec();
            needle.push(0);
            let at=bytes.windows(needle.len())
                .position(|part|part==needle.as_slice()).unwrap();
            if index>0{assert!(at>previous,"field {key} was out of Mako order");}
            previous=at;
        }
    }

    #[test]
    fn ordered_wire_retains_duplicate_keys_without_changing_dbus_signature(){
        use zbus::zvariant::{serialized::Context,LE,Type};
        assert_eq!(OrderedActionDict::SIGNATURE.to_string(),"a{ss}");
        assert_eq!(OrderedNotificationWire::SIGNATURE.to_string(),"a{sv}");
        let ctl=fixture();
        ctl.store.lock().unwrap().push(Request{
            actions:vec![
                Action{key:"z".into(),label:"Latest".into()},
                Action{key:"a".into(),label:"Middle".into()},
                Action{key:"z".into(),label:"Earliest".into()},
            ],
            ..Request::default()
        });
        let notes=ctl.ordered_snapshot(false).unwrap();
        assert!(!notes[0].fields.contains_key("actions"),
            "ordered method reply must not create a lossy temporary Dict");
        assert_eq!(notes[0].actions.0.len(),3);
        let wire=zbus::zvariant::to_bytes(Context::new_dbus(LE,0),&notes).unwrap();
        let bytes=wire.bytes();
        let find=|needle:&[u8]|bytes.windows(needle.len())
            .position(|segment|segment==needle).unwrap();
        assert!(find(b"Latest\0")<find(b"Middle\0"));
        assert!(find(b"Middle\0")<find(b"Earliest\0"));
        let (decoded,_):(Vec<HashMap<String,OwnedValue>>,_)=
            wire.deserialize().unwrap();
        assert_eq!(decoded.len(),1);
        assert!(decoded[0].contains_key("actions"));
    }

    #[test]
    fn action_dictionary_has_deterministic_key_order_and_documents_duplicate_collapse(){
        let control=fixture();
        control.store.lock().unwrap().push(Request{
            actions:vec![
                Action{key:"open".into(),label:"Second".into()},
                Action{key:"open".into(),label:"First".into()},
                Action{key:"dismiss".into(),label:"Ignore".into()},
            ],
            ..Request::default()
        });
        let notes=control.snapshot(false).unwrap();
        let value=&notes[0]["actions"];
        let Value::Dict(dict)=&**value else {panic!("actions must be a{{ss}}")};
        let pairs:Vec<_>=dict.iter().map(|(key,label)|{
            let (Value::Str(key),Value::Str(label))=(key,label) else{
                panic!("expected string action key and label");
            };
            (key.as_str().to_owned(),label.as_str().to_owned())
        }).collect();
        assert_eq!(pairs,vec![
            ("dismiss".into(),"Ignore".into()),
            ("open".into(),"First".into()),
        ]);
    }

    #[test]
    fn mode_change_reapplies_history_for_notifications_already_active() {
        let runtime=SharedRuntime::new(crate::config::Config::parse(
            "history=1\n[mode=quiet]\nhistory=0\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(id,std::time::Instant::now()).unwrap();
        let ctl=MakoControl::with_runtime(runtime.clone());
        ctl.set_modes_impl(vec!["quiet".into()]).unwrap();
        assert!(runtime.close(id,CloseReason::Dismissed,true).unwrap());
        assert!(runtime.store.lock().unwrap().history().is_empty());

        let id=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule(id,std::time::Instant::now()).unwrap();
        ctl.set_modes_impl(vec!["default".into()]).unwrap();
        assert!(runtime.close(id,CloseReason::Dismissed,true).unwrap());
        assert_eq!(runtime.store.lock().unwrap().history()[0].id,id);
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
    fn malformed_dismiss_option_types_reject_without_dismissing() {
        let control=fixture();
        let id=control.store.lock().unwrap().push(Request::default());
        for key in ["id","all","group","history"] {
            // Wrong-typed keys are not treated as missing/default values.
            let opts=[(key.to_string(),OwnedValue::from(-1i32))].into();
            assert!(matches!(control.dismiss_impl(&opts),
                Err(zbus::fdo::Error::InvalidArgs(_))),
                "{key} accepted a D-Bus signed integer of the wrong type");
            assert_eq!(control.store.lock().unwrap().active()[0].id,id);
        }
        // Unknown extension keys may be ignored, as in upstream Mako.
        let opts=[("future-option".to_string(),OwnedValue::from(true)),
                  ("id".into(),OwnedValue::from(id))].into();
        assert_eq!(control.dismiss_impl(&opts).unwrap(),vec![id]);
    }

    #[test]
    fn control_invoke_respects_actions_disabled_style() {
        let runtime=SharedRuntime::new(crate::config::Config::parse(
            "[category=warning]\nactions=0\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request{
            category:"warning".into(),
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],..Request::default()
        });
        let control=MakoControl::with_runtime(runtime);
        assert_eq!(control.find_action(id,"default").unwrap(),None);
        assert_eq!(control.find_action(0,"default").unwrap(),None);
        assert_eq!(control.find_action(id,"missing").unwrap(),None);
        assert_eq!(control.store.lock().unwrap().active()[0].actions.len(),1);
    }

    #[test]
    fn group_dismiss_keeps_unrelated_notifications_and_cancels_timers() {
        let runtime=SharedRuntime::new(crate::config::Config::parse(
            "group-by=app-name\ndefault-timeout=4000\n"
        ).unwrap());
        let control=MakoControl::with_runtime(runtime.clone());
        let now=std::time::Instant::now();
        let mut store=runtime.store.lock().unwrap();
        let first=store.push(Request{app_name:"alpha".into(),..Request::default()});
        let other=store.push(Request{app_name:"beta".into(),..Request::default()});
        let latest=store.push(Request{app_name:"alpha".into(),..Request::default()});
        drop(store);
        for id in [first,other,latest] {
            runtime.schedule_without_hook(id,now).unwrap();
        }
        let opts=[("group".into(),OwnedValue::from(true))].into();
        assert_eq!(control.dismiss_impl(&opts).unwrap(),vec![latest,first]);
        assert_eq!(runtime.store.lock().unwrap().active().len(),1);
        assert_eq!(runtime.store.lock().unwrap().active()[0].id,other);
        assert_eq!(runtime.store.lock().unwrap().history().len(),2);
        assert_eq!(runtime.expire(now+std::time::Duration::from_millis(4000))
            .unwrap(),vec![other]);
    }

    #[test]
    fn unsupported_group_criteria_fail_without_dismissing() {
        // The parser now rejects this Mako-invalid rule. Keep testing that
        // the control API fails safely if handed a malformed in-memory config.
        let mut config=crate::config::Config::default();
        config.global.insert("group-by".into(),"group-index".into());
        let runtime=SharedRuntime::new(config);
        let control=MakoControl::with_runtime(runtime.clone());
        runtime.store.lock().unwrap().push(Request::default());
        let opts=[("group".into(),OwnedValue::from(true))].into();
        assert!(control.dismiss_impl(&opts).is_err());
        assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    }

    #[test]
    fn duplicate_mode_names_are_discarded_in_first_occurrence_order() {
        let control=fixture();
        control.set_modes_impl(vec![
            "quiet".into(),"custom".into(),"quiet".into(),
            "default".into(),"custom".into(),
        ]).unwrap();
        assert_eq!(control.list_modes().unwrap(),
            vec!["quiet","custom","default"]);
        control.set_modes_impl(Vec::new()).unwrap();
        assert!(control.list_modes().unwrap().is_empty());
    }

    #[test]
    fn modes_can_be_set_without_implicitly_adding_default() {
        let control = fixture();
        control.set_modes_impl(vec!["quiet".into(), "custom".into()]).unwrap();
        assert_eq!(control.list_modes().unwrap(), vec!["quiet", "custom"]);
    }
}
