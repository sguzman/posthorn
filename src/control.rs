//! Mako control-interface implementation on Posthorn's isolated development
//! session-bus name. The public Mako method and property signatures are retained.
//! Group dismissal, modes, and transactional reload are implemented. The
//! Get/GetAll on the control path use ordered property serialization to retain
//! repeated action keys; production cutover remains an explicit MVP gate.

use std::collections::{HashMap,HashSet};
use std::sync::{Arc, Mutex};

use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use std::fmt;
use zbus::{interface, zvariant::{Dict, OwnedValue, SerializeValue, Signature, Value}, Connection};

use crate::dbus::{BUS_NAME, OBJECT_PATH};
use crate::notification::{CloseReason, Notification, NotificationStore};
use crate::runtime::SharedRuntime;

pub const CONTROL_INTERFACE: &str = "fr.emersion.Mako";
pub const CONTROL_PATH: &str = "/fr/emersion/Mako";
    
/// Mako parses DismissNotifications a{sv} in sender order. A HashMap at
/// the D-Bus boundary hides earlier malformed duplicate keys before the C
/// implementation would reject them. Keep the arrival sequence until every
/// known field has been type-checked.
#[derive(Debug, zbus::zvariant::Type)]
#[zvariant(signature = "a{sv}")]
struct OrderedDismissOptions(Vec<(String, OwnedValue)>);

impl<'de> Deserialize<'de> for OrderedDismissOptions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value = OrderedDismissOptions;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("arrival-ordered Mako a{sv} dismissal options")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, OwnedValue>()? {
                    entries.push((key, value));
                }
                Ok(OrderedDismissOptions(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

impl OrderedDismissOptions {
    fn into_effective_map(self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let mut options = HashMap::new();
        for (key, value) in self.0 {
            let valid = match key.as_str() {
                "id" => u32::try_from(&value).is_ok(),
                "all" | "group" | "history" => bool::try_from(&value).is_ok(),
                // Mako skips unknown value variants without interpreting them.
                _ => true,
            };
            if !valid {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "dismissal option '{key}' has incorrect D-Bus type"
                )));
            }
            // Mako overwrites repeated fields as it visits them; the last
            // valid value is effective, but an earlier invalid one is fatal.
            options.insert(key, value);
        }
        Ok(options)
    }
}

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

    fn dismiss_impl(&self, options: &HashMap<String, OwnedValue>) -> zbus::fdo::Result<Vec<(u32,bool)>> {
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
        let mut removed=Vec::new();
        for notification_id in ids {
            if let Some(runtime) = &self.runtime {
                if let Some(change)=runtime.close_with_change(
                    notification_id, CloseReason::Dismissed, history
                ).map_err(|e| zbus::fdo::Error::Failed(e.into()))? {
                    removed.push((notification_id,change));
                }
            } else if self.store.lock().map_err(|_|internal_error())?
                .close(notification_id,CloseReason::Dismissed,history,true).is_some(){
                removed.push((notification_id,true));
            }
        }
        Ok(removed)
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
    // This legacy/internal map snapshot intentionally collapses duplicate
    // keys. All public on-wire methods and standard Properties.Get/GetAll
    // serialize OrderedNotificationWire directly without this conversion.
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
        options: OrderedDismissOptions,
        #[zbus(connection)] conn: &Connection,
    ) -> zbus::fdo::Result<()> {
        let options = options.into_effective_map()?;
        let ids = self.dismiss_impl(&options)?;
        if !ids.is_empty() {
            for (id,changed) in ids {
                emit_notification_signal(conn, "NotificationClosed", id,
                    CloseReason::Dismissed as u32).await?;
                if changed {
                    crate::signals::invalidate(conn, "Notifications").await
                        .map_err(|e|zbus::fdo::Error::Failed(e.to_string()))?;
                }
            }
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
        if let (Some(runtime), Some(id)) = (&self.runtime, restored) {
            // Mako clears the restored style and reapplies criteria before
            // associating the new surface. Do not replay its Notify hooks or
            // start another expiration timer.
            runtime.associate_restored_surface(id)
                .map_err(zbus::fdo::Error::Failed)?;
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
        // zbus's generated property accessor requires Into<Value> and
        // collapses duplicate nested dictionary keys; this accessor is only
        // used for the internal macro interface. The control path overrides
        // standard Properties.Get/GetAll with OrderedControlProperties,
        // which uses ordered_snapshot instead.
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

/// zbus' default Properties helper and #[interface(property)] getter
/// convert the nested a{sv} value into OwnedValue. That materializes each
/// a{ss} as a BTreeMap, losing duplicate action keys that Mako 1.11 emits.
/// The control object therefore uses a path-local Properties implementation
/// that writes the known property values as typed variants without a Dict.
#[derive(Clone)]
pub struct OrderedControlProperties {
    control: MakoControl,
}

impl OrderedControlProperties {
    pub fn with_runtime(runtime: SharedRuntime) -> Self {
        Self { control: MakoControl::with_runtime(runtime) }
    }

    fn has_properties(name: &str) -> zbus::fdo::Result<bool> {
        match name {
            CONTROL_INTERFACE => Ok(true),
            // zbus registers these on every object path. They expose no
            // properties, so standard GetAll succeeds with an empty a{sv}.
            "org.freedesktop.DBus.Properties"
            | "org.freedesktop.DBus.Introspectable"
            | "org.freedesktop.DBus.Peer" => Ok(false),
            _ => Err(zbus::fdo::Error::UnknownInterface(name.into())),
        }
    }
}

/// Each Get reply is a D-Bus variant whose contained type depends on the
/// requested property. SerializeValue knows the inner signature and writes
/// the original ordered notification maps without any OwnedValue conversion.
#[derive(zbus::zvariant::Type)]
#[zvariant(signature = "v")]
enum OrderedPropertyValue {
    Modes(Vec<String>),
    Notifications(Vec<OrderedNotificationWire>),
}

impl Serialize for OrderedPropertyValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Modes(modes) => SerializeValue(modes).serialize(serializer),
            Self::Notifications(records) => SerializeValue(records).serialize(serializer),
        }
    }
}

#[derive(zbus::zvariant::Type)]
#[zvariant(signature = "a{sv}")]
struct OrderedControlPropertiesMap {
    modes: Vec<String>,
    notifications: Vec<OrderedNotificationWire>,
    empty: bool,
}

impl Serialize for OrderedControlPropertiesMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map=serializer.serialize_map(Some(if self.empty {0} else {2}))?;
        if !self.empty {
            map.serialize_entry("Modes",&SerializeValue(&self.modes))?;
            map.serialize_entry("Notifications",&SerializeValue(&self.notifications))?;
        }
        map.end()
    }
}

#[interface(name = "org.freedesktop.DBus.Properties")]
impl OrderedControlProperties {
    #[zbus(name = "Get")]
    fn get(&self, interface: &str, property: &str)
        -> zbus::fdo::Result<OrderedPropertyValue>
    {
        if !Self::has_properties(interface)? {
            return Err(zbus::fdo::Error::UnknownProperty(property.into()));
        }
        match property {
            "Modes" => Ok(OrderedPropertyValue::Modes(self.control.list_modes()?)),
            "Notifications" => Ok(OrderedPropertyValue::Notifications(
                self.control.ordered_snapshot(false)?)),
            _ => Err(zbus::fdo::Error::UnknownProperty(property.into())),
        }
    }

    #[zbus(name = "GetAll")]
    fn get_all(&self, interface: &str)
        -> zbus::fdo::Result<OrderedControlPropertiesMap>
    {
        if !Self::has_properties(interface)? {
            return Ok(OrderedControlPropertiesMap {
                modes:Vec::new(), notifications:Vec::new(), empty:true,
            });
        }
        Ok(OrderedControlPropertiesMap {
            modes:self.control.list_modes()?,
            notifications:self.control.ordered_snapshot(false)?,
            empty:false,
        })
    }

    #[zbus(name = "Set")]
    fn set(&self, interface: &str, property: &str, _value: OwnedValue)
        -> zbus::fdo::Result<()>
    {
        if !Self::has_properties(interface)? {
            return Err(zbus::fdo::Error::UnknownProperty(property.into()));
        }
        match property {
            "Modes" | "Notifications" =>
                Err(zbus::fdo::Error::PropertyReadOnly(property.into())),
            _ => Err(zbus::fdo::Error::UnknownProperty(property.into())),
        }
    }

    // The default zbus Properties interface declares this standard signal.
    // Retain it in introspection even though Posthorn already emits it with
    // its existing invalidation helper on the same path.
    #[zbus(signal)]
    async fn properties_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        interface_name: &str,
        changed_properties: HashMap<&str, Value<'_>>,
        invalidated_properties: Vec<&str>,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::{Request, Action};

    fn fixture() -> MakoControl {
        MakoControl::new(Arc::new(Mutex::new(NotificationStore::new(5))))
    }

    #[test]
    fn ordered_dismissal_options_validate_every_duplicate_before_collapsing() {
        use zbus::zvariant::Type;
        assert_eq!(OrderedDismissOptions::SIGNATURE.to_string(), "a{sv}");
        for key in ["id", "all", "group", "history"] {
            let correct = if key == "id" {
                OwnedValue::from(1u32)
            } else {
                OwnedValue::from(true)
            };
            let options = OrderedDismissOptions(vec![
                (key.into(), OwnedValue::from(-1i32)),
                (key.into(), correct),
            ]);
            assert!(matches!(options.into_effective_map(),
                Err(zbus::fdo::Error::InvalidArgs(_))),
                "malformed earlier {key} must not disappear");
        }
        let options = OrderedDismissOptions(vec![
            ("all".into(), OwnedValue::from(true)),
            ("future-extension".into(), OwnedValue::from(-1i32)),
            ("all".into(), OwnedValue::from(false)),
            ("id".into(), OwnedValue::from(7u32)),
        ]).into_effective_map().unwrap();
        assert!(!bool::try_from(&options["all"]).unwrap());
        assert_eq!(u32::try_from(&options["id"]).unwrap(), 7);
        assert!(options.contains_key("future-extension"));
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
        assert_eq!(control.dismiss_impl(&opts).unwrap(),vec![(id,true)]);
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
        assert_eq!(control.dismiss_impl(&opts).unwrap(),vec![(latest,true),(first,true)]);
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
