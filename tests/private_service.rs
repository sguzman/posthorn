#![cfg(feature = "dbus-integration")]
//! End-to-end D-Bus lifecycle through a safely isolated private name.

use std::collections::HashMap;
use std::time::Duration;
use posthorn::config::Config;
use posthorn::control::{CONTROL_INTERFACE, CONTROL_PATH};
use posthorn::dbus::{BUS_NAME, OBJECT_PATH};
use posthorn::service::{open_private_service, open_standard_service, tick_expirations, PRIVATE_BUS_NAME};
use zbus::{Proxy, zvariant::OwnedValue};

#[tokio::test]
async fn notification_bus_name_collision_never_queues_or_displaces_owner()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // This test is launched only inside the workflow's dbus-run-session.
    // No user-session org.freedesktop.Notifications ownership is touched.
    let (first,_) = open_standard_service(Config::default()).await?;
    let dbus = Proxy::new(&first,"org.freedesktop.DBus",
        "/org/freedesktop/DBus","org.freedesktop.DBus").await?;
    let owner_before:String=dbus.call("GetNameOwner",&(BUS_NAME,)).await?;
    assert_eq!(owner_before,first.unique_name().ok_or("missing owner name")?.as_str());
    let collision=open_standard_service(Config::default()).await;
    assert!(collision.is_err(),
        "second server must not replace or queue behind the existing owner");
    let owner_after:String=dbus.call("GetNameOwner",&(BUS_NAME,)).await?;
    assert_eq!(owner_after,owner_before,
        "collision must not displace the existing notification daemon");

    let (private,_) = open_private_service(Config::default()).await?;
    let private_dbus = Proxy::new(&private,"org.freedesktop.DBus",
        "/org/freedesktop/DBus","org.freedesktop.DBus").await?;
    let private_before:String=private_dbus.call(
        "GetNameOwner",&(PRIVATE_BUS_NAME,)).await?;
    assert!(open_private_service(Config::default()).await.is_err(),
        "private development name must have the same no-queue guarantee");
    let private_after:String=private_dbus.call(
        "GetNameOwner",&(PRIVATE_BUS_NAME,)).await?;
    assert_eq!(private_before,private_after);
    Ok(())
}

#[tokio::test]
async fn private_service_expires_and_archives_a_notification() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::parse(
        "default-timeout= 35\nignore-timeout=1\nmax-history=3\n"
    )?;
    let (conn, runtime) = open_private_service(config).await?;
    let desktop = Proxy::new(&conn, PRIVATE_BUS_NAME, OBJECT_PATH, BUS_NAME).await?;
    let control = Proxy::new(&conn, PRIVATE_BUS_NAME, CONTROL_PATH, CONTROL_INTERFACE).await?;
    let id: u32 = desktop.call("Notify", &(
        "test-sender", 0u32, "", "Done", "body", Vec::<String>::new(),
        HashMap::<String, OwnedValue>::new(), 100000i32
    )).await?;
    let original_instance=runtime.store.lock().unwrap()
        .active_instance(id).ok_or("new D-Bus notification lacks an active instance")?;
    assert!(runtime.next_deadline()?.is_some());

    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(tick_expirations(&conn, &runtime).await?, 1);
    assert_eq!(tick_expirations(&conn, &runtime).await?, 0);

    let current: Vec<HashMap<String, OwnedValue>> =
        control.call("ListNotifications", &()).await?;
    let history: Vec<HashMap<String, OwnedValue>> =
        control.call("ListHistory", &()).await?;
    assert!(current.is_empty());
    assert_eq!(history.len(), 1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?, id);
    assert!(runtime.store.lock().unwrap().active_instance(id).is_none(),
        "expired notifications must not remain active click targets");

    // Exercise a separate CLI session-bus connection, not just the server
    // internals or a proxy sharing the same connection.
    use posthorn::ctl::{Command, execute};
    execute(Command::Modes { edit: vec![('s', "quiet".into())] }).await?;
    let modes: Vec<String> = control.call("ListModes", &()).await?;
    assert_eq!(modes, vec!["quiet"]);
    execute(Command::Restore).await?;
    let restored: Vec<HashMap<String, OwnedValue>> =
        control.call("ListNotifications", &()).await?;
    assert_eq!(restored.len(), 1);
    assert_ne!(runtime.store.lock().unwrap().active_instance(id),Some(original_instance),
        "restore over the control bus must start a new active click lifetime");
    assert!(runtime.next_deadline()?.is_none(),
        "upstream Mako restores notifications without new expiration timers");
    execute(Command::Dismiss {
        id, all: false, group: false, history: false
    }).await?;
    let dismissed: Vec<HashMap<String, OwnedValue>> =
        control.call("ListNotifications", &()).await?;
    assert!(dismissed.is_empty());
    assert!(runtime.next_deadline()?.is_none(),
        "dismissed notification must not retain an expiration timer");
    Ok(())
}

#[tokio::test]
async fn private_dbus_image_path_preserves_raw_linux_file_url_through_cairo()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use std::os::unix::ffi::OsStringExt;
    use posthorn::render::render_notification;
    use posthorn::config::Options;
    let dir=tempfile::tempdir()?;
    let filename=std::ffi::OsString::from_vec(b"dbus-hint-icon\xff.png".to_vec());
    let target=dir.path().join(filename);
    let icon=gdk_pixbuf::Pixbuf::new(
        gdk_pixbuf::Colorspace::Rgb,true,8,12,12
    ).ok_or("unable to allocate image fixture")?;
    icon.fill(0xFF0000FF);
    icon.savev(&target,"png",&[])?;
    let uri=format!("file://{}/dbus-hint-icon%FF.png",dir.path().display());

    let (conn,runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let hints:HashMap<String,OwnedValue>=[(
        "image-path".to_owned(),
        OwnedValue::try_from(zbus::zvariant::Value::from(uri.clone()))?,
    )].into();
    let id:u32=desktop.call("Notify",&(
        "icon-dbus-test",0u32,"missing.png","Icon","",
        Vec::<String>::new(),hints,-1i32
    )).await?;
    let note=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).cloned().ok_or("missing notification")?;
    assert_eq!(note.app_icon,uri);
    assert!(note.image_data.is_none());

    let mut style=Options::new();
    for (key,value) in [
        ("width","160"),("height","70"),("padding","4"),
        ("border-size","0"),("icon-location","left"),
        ("max-icon-size","16"),("background-color","#305747"),
    ] {
        style.insert(key.into(),value.into());
    }
    let frame=render_notification(&note,&style)?;
    let at=(frame.height as usize/2)*frame.stride as usize+6*4;
    assert_eq!(&frame.pixels[at..at+4],&[0,0,255,255],
        "image-path must flow from Notify through raw-byte URL decoder into Cairo");
    Ok(())
}

#[tokio::test]
async fn empty_image_path_hint_clears_app_icon_through_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let hints:HashMap<String,OwnedValue>=[(
        "image-path".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from(""))?,
    )].into();
    let id:u32=desktop.call("Notify",&(
        "image-hint-fixture",0u32,"original.png","Summary","",
        Vec::<String>::new(),hints,-1i32
    )).await?;
    let note=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().clone();
    assert_eq!(note.app_icon,"",
        "Mako accepts an empty image-path as an override, not as absent");

    let legacy:HashMap<String,OwnedValue>=[(
        "image_path".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("legacy.png"))?,
    )].into();
    let replacement:u32=desktop.call("Notify",&(
        "image-hint-fixture",id,"original.png","Replacement","",
        Vec::<String>::new(),legacy,-1i32
    )).await?;
    assert_eq!(replacement,id);
    let note=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().clone();
    assert_eq!(note.app_icon,"legacy.png");
    Ok(())
}

/// A synthetic raw a{sv} sender can preserve duplicate keys and alias
/// arrival order; HashMap-based test clients cannot exercise that path.
#[derive(zbus::zvariant::Type)]
#[zvariant(signature="a{sv}")]
struct WireOrderedHints(Vec<(String,OwnedValue)>);

impl serde::Serialize for WireOrderedHints {
    fn serialize<S:serde::Serializer>(&self,serializer:S)
        ->Result<S::Ok,S::Error>
    {
        use serde::ser::SerializeMap;
        let mut map=serializer.serialize_map(Some(self.0.len()))?;
        for (key,value) in &self.0{
            map.serialize_entry(key,value)?;
        }
        map.end()
    }
}

#[tokio::test]
async fn ordered_notify_hints_use_last_wire_alias_and_reject_invalid_prior_entry()
    ->Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let txt=|s:&str|OwnedValue::try_from(zbus::zvariant::Value::from(s)).unwrap();
    let hints=WireOrderedHints(vec![
        ("image_path".into(),txt("first-icon")),
        ("image-path".into(),txt("")),
        ("x-dunst-stack-tag".into(),txt("first-tag")),
        ("x-canonical-private-synchronous".into(),txt("last-tag")),
        ("urgency".into(),OwnedValue::from(0u8)),
        ("urgency".into(),OwnedValue::from(2u8)),
    ]);
    let id:u32=desktop.call("Notify",&(
        "ordered-hints",0u32,"argument-icon","Title","",
        Vec::<String>::new(),hints,-1i32
    )).await?;
    let note=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().clone();
    assert_eq!(note.app_icon,"");
    assert_eq!(note.tag,"last-tag");
    assert_eq!(note.urgency,posthorn::notification::Urgency::Critical);

    let reverse=WireOrderedHints(vec![
        ("image-path".into(),txt("modern")),
        ("image_path".into(),txt("legacy-last")),
        ("x-canonical-private-synchronous".into(),txt("first-tag")),
        ("x-dunst-stack-tag".into(),txt("newest-tag")),
    ]);
    let same:u32=desktop.call("Notify",&(
        "ordered-hints",id,"argument-icon","Changed","",
        Vec::<String>::new(),reverse,-1i32
    )).await?;
    assert_eq!(same,id);
    let note=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().clone();
    assert_eq!(note.app_icon,"legacy-last");
    assert_eq!(note.tag,"newest-tag");

    let invalid=WireOrderedHints(vec![
        ("image_path".into(),OwnedValue::from(123i32)),
        ("image-path".into(),txt("safe-later")),
    ]);
    let bad:zbus::Result<u32>=desktop.call("Notify",&(
        "ordered-hints",id,"argument-icon","Do not replace","",
        Vec::<String>::new(),invalid,-1i32
    )).await;
    assert!(bad.is_err(),"Mako rejects a malformed earlier hint");
    assert_eq!(runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().summary,"Changed");
    Ok(())
}

#[tokio::test]
async fn empty_timeout_and_lp64_history_narrowing_reach_private_dbus()
    ->Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let config=Config::parse(
        "default-timeout=\nignore-timeout=1\nmax-history=4294967297\n"
    )?;
    assert_eq!(config.max_history,1);
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let mut ids=Vec::new();
    for num in 0..2 {
        let id:u32=desktop.call("Notify",&(
            "strtol-parity",0u32,"",format!("Notice {num}"),"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),10i32
        )).await?;
        assert!(runtime.next_deadline()?.is_none(),
            "empty default-timeout is zero, so ignore-timeout makes it persistent");
        ids.push(id);
        let options:HashMap<String,OwnedValue>=[(
            "id".into(),OwnedValue::from(id)
        )].into();
        let _:()=ctl.call("DismissNotifications",&(options,)).await?;
    }
    let history:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListHistory",&()).await?;
    assert_eq!(history.len(),1,
        "LP64 max-history wraps 4294967297 to signed 1");
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?,ids[1]);
    Ok(())
}

#[tokio::test]
async fn malformed_dismiss_options_are_rejected_over_real_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let id:u32=desktop.call("Notify",&(
        "typed-options-smoke",0u32,"","Keep me","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    for key in ["id","all","group","history"] {
        let malformed:HashMap<String,OwnedValue>=[
            (key.into(),OwnedValue::from(-1i32))
        ].into();
        let result:zbus::Result<()> =
            control.call("DismissNotifications",&(malformed,)).await;
        assert!(result.is_err(),"malformed {key} option unexpectedly succeeded");
        let current:Vec<HashMap<String,OwnedValue>>=
            control.call("ListNotifications",&()).await?;
        assert_eq!(current.len(),1,
            "bad {key} option dismissed an unrelated notification");
        assert_eq!(u32::try_from(current[0].get("id").unwrap())?,id);
    }
    let valid:HashMap<String,OwnedValue>=[
        ("id".into(),OwnedValue::from(id))
    ].into();
    let _:()=control.call("DismissNotifications",&(valid,)).await?;
    let current:Vec<HashMap<String,OwnedValue>>=
        control.call("ListNotifications",&()).await?;
    assert!(current.is_empty());
    Ok(())
}

/// Preserve duplicate a{sv} options on the test wire, as sd-bus allows.
/// HashMap would silently merge the keys before they reach the daemon.
#[derive(zbus::zvariant::Type)]
#[zvariant(signature = "a{sv}")]
struct OrderedDismissWire(Vec<(String, OwnedValue)>);

impl serde::Serialize for OrderedDismissWire {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[tokio::test]
async fn duplicate_dismiss_keys_follow_mako_validation_on_private_dbus()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let (conn, runtime) = open_private_service(Config::default()).await?;
    let desktop = Proxy::new(&conn, PRIVATE_BUS_NAME, OBJECT_PATH, BUS_NAME).await?;
    let control = Proxy::new(&conn, PRIVATE_BUS_NAME, CONTROL_PATH, CONTROL_INTERFACE).await?;
    let id: u32 = desktop.call("Notify", &(
        "duplicate-dismiss-wire", 0u32, "", "Keep this", "",
        Vec::<String>::new(), HashMap::<String, OwnedValue>::new(), -1i32
    )).await?;

    // The first (badly typed) id must fail, even when a later id is valid.
    let bad_id = OrderedDismissWire(vec![
        ("id".into(), OwnedValue::from(-1i32)),
        ("id".into(), OwnedValue::from(id)),
    ]);
    let result: zbus::Result<()> = control.call("DismissNotifications", &(bad_id,)).await;
    assert!(result.is_err(), "earlier invalid id was hidden by its duplicate");
    assert_eq!(runtime.store.lock().unwrap().active().len(), 1);

    for key in ["all", "group", "history"] {
        let bad = OrderedDismissWire(vec![
            (key.into(), OwnedValue::from(-1i32)),
            (key.into(), OwnedValue::from(false)),
            ("id".into(), OwnedValue::from(id)),
        ]);
        let result: zbus::Result<()> = control.call("DismissNotifications", &(bad,)).await;
        assert!(result.is_err(), "earlier invalid {key} was hidden by its duplicate");
        assert_eq!(runtime.store.lock().unwrap().active().len(), 1);
    }

    // A valid later duplicate overrides the first, like Mako's sequential
    // sd_bus_message_read loop. Unknown option values are skipped.
    let valid = OrderedDismissWire(vec![
        ("all".into(), OwnedValue::from(true)),
        ("unused-extension".into(), OwnedValue::from(-1i32)),
        ("all".into(), OwnedValue::from(false)),
        ("id".into(), OwnedValue::from(id)),
        ("history".into(), OwnedValue::from(false)),
    ]);
    let _: () = control.call("DismissNotifications", &(valid,)).await?;
    assert!(runtime.store.lock().unwrap().active().is_empty());
    assert!(runtime.store.lock().unwrap().history().is_empty());
    Ok(())
}

#[tokio::test]
async fn private_control_menu_invokes_existing_action_without_shell()
    -> Result<(),Box<dyn std::error::Error + Send + Sync>>
{
    use posthorn::ctl::{execute,Command};
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let id:u32=desktop.call("Notify",&(
        "menu-fixture",0u32,"","Title","Body",
        vec!["default".to_owned(),"Open".to_owned()],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert!(id>0);
    execute(Command::Menu{id,program:vec!["cat".into()]}).await?;
    Ok(())
}

/// Mako's sd-bus a{ss} writer permits repeated keys in arrival order.
/// Typed zvariant Dict readers collapse duplicates; inspect actual body bytes.
#[tokio::test]
async fn control_action_dictionary_preserves_raw_wire_order_with_duplicate_keys()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let id:u32=desktop.call("Notify",&(
        "ordered-wire-test",0u32,"","Unique summary","",
        vec![
            "duplicate".to_string(),"Earliest-option-1".to_string(),
            "middle".to_string(),"Middle-option-2".to_string(),
            "duplicate".to_string(),"Latest-option-3".to_string(),
        ],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let check=|bytes:&[u8]|{
        let at=|needle:&[u8]|bytes.windows(needle.len()).position(|entry|entry==needle)
            .expect("action label missing from a{ss} wire data");
        assert!(at(b"Latest-option-3\0")<at(b"Middle-option-2\0"));
        assert!(at(b"Middle-option-2\0")<at(b"Earliest-option-1\0"));
        // Mako's outer a{sv} fields also have a fixed source-level order.
        let mut previous=None;
        for name in ["app-name","app-icon","category","desktop-entry",
                     "summary","body","id","urgency","actions"] {
            let mut needle=name.as_bytes().to_vec();
            needle.push(0);
            let position=at(&needle);
            if let Some(earlier)=previous{
                assert!(earlier<position,"wire field {name} was out of Mako order");
            }
            previous=Some(position);
        }
    };
    let reply=ctl.call_method("ListNotifications",&()).await?;
    let body=reply.body();
    assert_eq!(body.signature().to_string(),"aa{sv}");
    check(body.data().bytes());
    let records:Vec<HashMap<String,OwnedValue>>=body.deserialize()?;
    assert_eq!(records.len(),1);
    assert_eq!(u32::try_from(records[0].get("id").unwrap())?,id);

    // The control path now uses a custom standard Properties adapter so
    // both Get(v) and GetAll(a{sv}) preserve the same duplicate keys and
    // entry ordering as ListNotifications. Check raw bytes before zvariant
    // turns the dictionary into a key-collapsing HashMap.
    let props=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,
        "org.freedesktop.DBus.Properties").await?;
    let reply=props.call_method("Get",&(CONTROL_INTERFACE,"Notifications")).await?;
    assert_eq!(reply.body().signature().to_string(),"v");
    check(reply.body().data().bytes());
    let all=props.call_method("GetAll",&(CONTROL_INTERFACE,)).await?;
    assert_eq!(all.body().signature().to_string(),"a{sv}");
    check(all.body().data().bytes());
    let introspect=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,
        "org.freedesktop.DBus.Introspectable").await?;
    let xml:String=introspect.call("Introspect",&()).await?;
    assert!(xml.contains("name=\"PropertiesChanged\""),
        "the replacement standard Properties interface must still advertise its signal");
    let all_body=all.body();
    let all_bytes=all_body.data().bytes();
    assert!(all_bytes.windows(b"Modes\0".len()).any(|w|w==b"Modes\0"),
        "standard Properties.GetAll must include Modes as well");
    // Regular applications decode the property map rather than inspecting
    // bytes. Confirm that both are still present under typed GetAll.
    let decoded:HashMap<String,OwnedValue>=all_body.deserialize()?;
    assert_eq!(decoded.len(),2);
    assert!(decoded.contains_key("Notifications"));
    assert!(decoded.contains_key("Modes"));
    let _:()=ctl.call("SetModes",&(vec!["quiet".to_string()],)).await?;
    let modes_reply=props.call_method("Get",&(CONTROL_INTERFACE,"Modes")).await?;
    assert_eq!(modes_reply.body().signature().to_string(),"v");
    assert!(modes_reply.body().data().bytes()
        .windows(b"quiet\0".len()).any(|w|w==b"quiet\0"),
        "ordinary Properties.Get must observe the updated mode list");
    for known in [
        "org.freedesktop.DBus.Introspectable",
        "org.freedesktop.DBus.Peer",
        "org.freedesktop.DBus.Properties",
    ] {
        let empty:HashMap<String,OwnedValue>=props.call("GetAll",&(known,)).await?;
        assert!(empty.is_empty(),"{known} has no standard properties");
    }
    let unknown_all:zbus::Result<HashMap<String,OwnedValue>>=
        props.call("GetAll",&("no.such.Interface",)).await;
    assert!(unknown_all.is_err(),"unknown interface must not return an empty map");
    let missing:zbus::Result<OwnedValue>=props.call("Get",&(
        CONTROL_INTERFACE,"NotAProperty"
    )).await;
    assert!(matches!(missing,Err(zbus::Error::MethodError(_,_,_))),
        "standard Properties.Get must reject unknown properties");
    let read_only:zbus::Result<()>=props.call("Set",&(
        CONTROL_INTERFACE,"Modes",OwnedValue::from(true)
    )).await;
    assert!(read_only.is_err(),"Mako control properties are read-only");
    let other:zbus::Result<OwnedValue>=props.call("Get",&(
        "no.such.Interface","Modes"
    )).await;
    assert!(other.is_err(),"standard Properties must reject unknown interface");

    let options:HashMap<String,OwnedValue>=[
        ("id".into(),OwnedValue::from(id))
    ].into();
    let _:()=ctl.call("DismissNotifications",&(options,)).await?;
    let history=ctl.call_method("ListHistory",&()).await?;
    assert_eq!(history.body().signature().to_string(),"aa{sv}");
    check(history.body().data().bytes());
    Ok(())
}

#[tokio::test]
async fn action_dictionary_replacement_keeps_only_new_ordered_entries_in_history()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let id:u32=desktop.call("Notify",&(
        "ordered-replacement",0u32,"","Original","",
        vec![
            "old-z".to_string(),"Original-first".to_string(),
            "old-a".to_string(),"Original-last".to_string(),
        ],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let replacement:u32=desktop.call("Notify",&(
        "ordered-replacement",id,"","Replaced","",
        vec![
            "new-z".to_string(),"Replacement-first".to_string(),
            "new-a".to_string(),"Replacement-second".to_string(),
            "new-z".to_string(),"Replacement-last".to_string(),
        ],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_eq!(replacement,id);
    let assert_actions=|bytes:&[u8]|{
        let find=|needle:&[u8]|bytes.windows(needle.len())
            .position(|part|part==needle).expect("replacement action absent");
        assert!(find(b"Replacement-last\0")<find(b"Replacement-second\0"));
        assert!(find(b"Replacement-second\0")<find(b"Replacement-first\0"));
        assert!(!bytes.windows(b"Original-first\0".len())
            .any(|part|part==b"Original-first\0"),
            "obsolete replaced action leaked into control wire");
    };
    let active=ctl.call_method("ListNotifications",&()).await?;
    assert_actions(active.body().data().bytes());
    let opts:HashMap<String,OwnedValue>=[("id".into(),OwnedValue::from(id))].into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let history=ctl.call_method("ListHistory",&()).await?;
    assert_actions(history.body().data().bytes());
    Ok(())
}

#[tokio::test]
async fn posthornctl_menu_uses_mako_sender_reversed_action_order()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use futures_util::StreamExt;
    use posthorn::ctl::{execute,Command};
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let mut signals=desktop.receive_all_signals().await?;
    let id:u32=desktop.call("Notify",&(
        "menu-order",0u32,"","Menu order","",
        vec![
            "first".to_string(),"Early label".to_string(),
            "middle".to_string(),"Middle label".to_string(),
            "last".to_string(),"Latest label".to_string(),
        ],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    execute(Command::Menu{id,program:vec!["cat".into()]}).await?;
    let received=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("menu action signal not emitted")?;
    assert_eq!(received.header().member().unwrap().as_str(),"ActionInvoked");
    let (actual_id,key):(u32,String)=received.body().deserialize()?;
    assert_eq!((actual_id,key),(id,"last".to_string()),
        "makoctl menu must select the first title in linked-list order");
    Ok(())
}

#[tokio::test]
async fn menu_selects_valid_first_line_even_with_non_utf8_trailing_stdout()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use futures_util::StreamExt;
    use posthorn::ctl::{execute,Command};
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let mut signals=desktop.receive_all_signals().await?;
    let id:u32=desktop.call("Notify",&(
        "menu-utf8",0u32,"","Menu output","",
        vec!["confirm".to_owned(),"Open".to_owned()],
        HashMap::<String,OwnedValue>::new(),-1i32,
    )).await?;
    execute(Command::Menu {
        id,
        program:vec![
            "sh".into(),"-c".into(),"printf 'Open\\n\\377\\n'".into()
        ],
    }).await?;
    let message=tokio::time::timeout(std::time::Duration::from_secs(3),
        signals.next()).await?.ok_or("expected ActionInvoked signal")?;
    assert_eq!(message.header().member().unwrap().as_str(),"ActionInvoked");
    let invoked:(u32,String)=message.body().deserialize()?;
    assert_eq!(invoked,(id,"confirm".to_owned()),
        "makoctl getline must ignore non-UTF8 bytes after the first line");
    Ok(())
}

#[tokio::test]
async fn action_dictionary_wire_conversion_has_deterministic_keys()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use zbus::zvariant::Value;
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let id:u32=desktop.call("Notify",&(
        "ordered-actions",0u32,"","Menu items","",
        vec![
            "open".to_string(),"Open early".to_string(),
            "dismiss".to_string(),"Ignore".to_string(),
            "open".to_string(),"Open late".to_string(),
        ],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let notes:Vec<HashMap<String,OwnedValue>>=
        control.call("ListNotifications",&()).await?;
    assert_eq!(notes.len(),1);
    assert_eq!(u32::try_from(notes[0].get("id").unwrap())?,id);
    let actions=notes[0].get("actions").unwrap();
    let Value::Dict(dict)=&**actions else {panic!("expected action dictionary")};
    let actual:Vec<_>=dict.iter().map(|(key,label)|{
        let (Value::Str(key),Value::Str(label))=(key,label) else{
            panic!("malformed action entry")
        };
        (key.as_str().to_owned(),label.as_str().to_owned())
    }).collect();
    assert_eq!(actual,vec![
        ("dismiss".into(),"Ignore".into()),
        ("open".into(),"Open early".into()),
    ]);
    let text=posthorn::ctl::notification_text(&notes[0]);
    assert!(text.contains(
        "  Actions:\n    dismiss: Ignore\n    open: Open early\n"
    ),"{text}");
    Ok(())
}

#[tokio::test]
async fn overlapping_different_width_hotspots_dispatch_to_lower_dbus_notification()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::desktop::{visible_members_indexed,style_for_note_index};
    use posthorn::input::{dispatch_click,hit_test};
    use posthorn::wayland::PointerClick;
    let cfg=Config::parse(
        "width=160\nheight=90\nmargin=0,0,-20,0\n         on-button-right=dismiss\n         [category=top]\nwidth=80\n         [category=lower]\nmargin=-20,0,0,0\n"
    )?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let send=|category:&str|{
        let hints:HashMap<String,OwnedValue>=[(
            "category".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from(category)).unwrap()
        )].into();
        (
            "overlap-click-fixture",0u32,"",category.to_owned(),"",
            Vec::<String>::new(),hints,-1i32
        )
    };
    let lower:u32=desktop.call("Notify",&send("lower")).await?;
    let upper:u32=desktop.call("Notify",&send("top")).await?;
    let members=visible_members_indexed(&runtime)?;
    assert_eq!(members[0].notification.id,upper);
    let style=style_for_note_index(
        &runtime,&members[0].notification,&["default".to_owned()],
        members[0].group_index
    );
    let height=posthorn::render::measure_notification_height_group(
        &members[0].notification,&style,0
    )?;
    let click=PointerClick{x:20.0,y:f64::from(height)-10.0,button:273,serial:29};
    assert_eq!(hit_test(&runtime,click)?.map(|(id,_)|id),Some(lower),
        "wider lower popup must remain hittable outside narrow top X hotspot");
    dispatch_click(&conn,&runtime,click).await?;
    let active:Vec<HashMap<String,OwnedValue>>=ctl.call("ListNotifications",&()).await?;
    assert_eq!(active.len(),1);
    assert_eq!(u32::try_from(active[0].get("id").unwrap())?,upper);
    let history:Vec<HashMap<String,OwnedValue>>=ctl.call("ListHistory",&()).await?;
    assert_eq!(history.len(),1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?,lower);
    Ok(())
}

#[tokio::test]
async fn pointer_button_bindings_dispatch_through_private_desktop_bus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::input::dispatch_click;
    use posthorn::wayland::PointerClick;
    let config=Config::parse(
        "width=160\nheight=90\nmargin=6\n         on-button-left=invoke-default-action\n         on-button-right=dismiss\n         default-timeout=0\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let id:u32=desktop.call("Notify",&(
        "input-fixture",0u32,"","Button","Visible",
        vec!["default".to_string(),"Open".to_string()],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    dispatch_click(&conn,&runtime,PointerClick{x:30.0,y:20.0,button:272,serial:21}).await?;
    let current:Vec<HashMap<String,OwnedValue>>=
        control.call("ListNotifications",&()).await?;
    let history:Vec<HashMap<String,OwnedValue>>=
        control.call("ListHistory",&()).await?;
    assert!(current.is_empty());
    assert_eq!(history.len(),1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?,id);

    let second:u32=desktop.call("Notify",&(
        "input-fixture",0u32,"","Second","Visible",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_ne!(id,second);
    dispatch_click(&conn,&runtime,PointerClick{x:30.0,y:20.0,button:273,serial:22}).await?;
    let current:Vec<HashMap<String,OwnedValue>>=
        control.call("ListNotifications",&()).await?;
    assert!(current.is_empty());

    // A transparent margin must never dismiss the third notification.
    let _:u32=desktop.call("Notify",&(
        "input-fixture",0u32,"","Third","Visible",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    dispatch_click(&conn,&runtime,PointerClick{x:30.0,y:90.0,button:273,serial:23}).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    Ok(())
}

#[tokio::test]
async fn makoctl_mode_set_positional_arguments_reach_private_bus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::ctl::{execute,parse_command};
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let args=["mode","-s","quiet","reading"]
        .into_iter().map(str::to_owned).collect::<Vec<_>>();
    execute(parse_command(&args)?).await?;
    let selected:Vec<String>=control.call("ListModes",&()).await?;
    assert_eq!(selected,vec!["quiet","reading"]);
    let args=["mode","-s"].into_iter().map(str::to_owned).collect::<Vec<_>>();
    execute(parse_command(&args)?).await?;
    let cleared:Vec<String>=control.call("ListModes",&()).await?;
    assert!(cleared.is_empty(),"makoctl -s must allow the empty mode set");
    Ok(())
}

#[tokio::test]
async fn scoped_group_dismissal_uses_selected_notification_criteria_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::hooks::NotifyBinding;
    use posthorn::input::dispatch_binding;

    let config=Config::parse(
        "group-by=none\n[category=warning]\ngroup-by=app-name\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let notify=|app:&'static str, category:&'static str| {
        let mut hints=HashMap::<String,OwnedValue>::new();
        hints.insert("category".into(),OwnedValue::try_from(zbus::zvariant::Value::from(category)).unwrap());
        (app,hints)
    };
    let (app,hints)=notify("sample-app","success");
    let ordinary:u32=desktop.call("Notify",&(
        app,0u32,"","ordinary","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=notify("independent","success");
    let unrelated:u32=desktop.call("Notify",&(
        app,0u32,"","independent","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=notify("sample-app","warning");
    let warning:u32=desktop.call("Notify",&(
        app,0u32,"","warning","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let opts:HashMap<String,OwnedValue>=[
        ("group".into(),OwnedValue::from(true)),
    ].into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let active:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    assert_eq!(active.len(),1);
    assert_eq!(u32::try_from(active[0].get("id").unwrap())?,unrelated);
    let archived:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListHistory",&()).await?;
    assert!(archived.iter().any(|m|u32::try_from(m.get("id").unwrap()).ok()==Some(ordinary)));
    assert!(archived.iter().any(|m|u32::try_from(m.get("id").unwrap()).ok()==Some(warning)));

    let (app,hints)=notify("sample-app","success");
    let ordinary:u32=desktop.call("Notify",&(
        app,0u32,"","ordinary again","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=notify("sample-app","warning");
    let warning:u32=desktop.call("Notify",&(
        app,0u32,"","warning again","",Vec::<String>::new(),hints,-1i32
    )).await?;
    dispatch_binding(&conn,&runtime,warning,NotifyBinding::DismissGroup,None).await?;
    let active:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    assert_eq!(active.len(),1);
    assert_eq!(u32::try_from(active[0].get("id").unwrap())?,unrelated);
    assert!(runtime.store.lock().unwrap().active().iter().all(|n|n.id!=ordinary));
    Ok(())
}

#[tokio::test]
async fn ascending_sort_groups_after_later_scoped_warning_over_private_bus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::desktop::visible_members_indexed;
    use posthorn::hooks::NotifyBinding;
    use posthorn::input::dispatch_binding;
    let config=Config::parse(
        "sort=+time\ngroup-by=none\n[category=warning]\ngroup-by=app-name\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let send=|app:&str,category:&str| {
        let hint:HashMap<String,OwnedValue>=[(
            "category".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from(category)).unwrap()
        )].into();
        (app.to_owned(),hint)
    };
    let (app,hints)=send("same","success");
    let old:u32=desktop.call("Notify",&(
        app,0u32,"","Old","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=send("different","success");
    let other:u32=desktop.call("Notify",&(
        app,0u32,"","Other","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=send("same","warning");
    let newest:u32=desktop.call("Notify",&(
        app,0u32,"","Warning","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let visible=visible_members_indexed(&runtime)?;
    assert_eq!(visible.iter().map(|v|(v.notification.id,v.group_count))
        .collect::<Vec<_>>(),vec![(old,2),(other,1)],
        "later scoped warning must regroup earlier plain notification at +time");
    let listed:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    assert_eq!(listed.iter().map(|m|
        u32::try_from(m.get("id").unwrap()).unwrap()).collect::<Vec<_>>(),
        vec![old,newest,other],
        "Mako moves grouped notifications in the actual active D-Bus list");
    dispatch_binding(&conn,&runtime,newest,NotifyBinding::DismissGroup,None).await?;
    let active:Vec<HashMap<String,OwnedValue>>=ctl.call("ListNotifications",&()).await?;
    assert_eq!(active.len(),1);
    assert_eq!(u32::try_from(active[0].get("id").unwrap())?,other);
    Ok(())
}

#[tokio::test]
async fn removing_group_trigger_regroups_survivors_over_private_bus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::desktop::visible_members_indexed;
    let config=Config::parse(
        "group-by=none\n[category=warning]\ngroup-by=app-name\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let send=|app:&str,category:&str| {
        let hints:HashMap<String,OwnedValue>=[(
            "category".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from(category)).unwrap()
        )].into();
        (app.to_owned(),hints)
    };
    let (app,hints)=send("same","success");
    let first:u32=desktop.call("Notify",&(
        app,0u32,"","First","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=send("same","warning");
    let trigger:u32=desktop.call("Notify",&(
        app,0u32,"","Group trigger","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=send("different","success");
    let other:u32=desktop.call("Notify",&(
        app,0u32,"","Other","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let (app,hints)=send("same","success");
    let newest:u32=desktop.call("Notify",&(
        app,0u32,"","Newest","",Vec::<String>::new(),hints,-1i32
    )).await?;
    let _:()=desktop.call("CloseNotification",&(trigger,)).await?;
    let active:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    assert_eq!(active.iter().map(|record|
        u32::try_from(record.get("id").unwrap()).unwrap()).collect::<Vec<_>>(),
        vec![newest,first,other],
        "Mako uses the removed trigger's group-by to reorder survivors");
    let visible=visible_members_indexed(&runtime)?;
    assert_eq!(visible.iter().map(|n|(n.notification.id,n.group_count))
        .collect::<Vec<_>>(),vec![(newest,2),(other,1)]);
    Ok(())
}

#[tokio::test]
async fn category_exec_hook_and_quiet_mode_are_isolated_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let dir=tempfile::tempdir()?;
    let output=dir.path().join("category-hook-events");
    let config=Config::parse(&format!(
        "[category=warning]\non-notify=exec printf '%s,' \"$id\" >> '{}'\n[mode=quiet]\non-notify=none\n",
        output.display()
    ))?;
    let (conn,_runtime)=open_private_service(config).await?;
    let dest=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let send=|category:&str|{
        let hints:HashMap<String,OwnedValue>=[(
            "category".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from(category)).unwrap()
        )].into();
        ("category-hook-fixture",0u32,"","Category","",
         Vec::<String>::new(),hints,-1i32)
    };
    let _:u32=dest.call("Notify",&send("success")).await?;
    assert!(!output.exists(),"ordinary category unexpectedly ran warning hook");
    let warning:u32=dest.call("Notify",&send("warning")).await?;
    tokio::time::timeout(std::time::Duration::from_secs(3),async{
        loop{
            if std::fs::read_to_string(&output).unwrap_or_default()
                ==format!("{warning},"){break;}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await?;
    let _:()=ctl.call("SetMode",&("quiet",)).await?;
    let _:u32=dest.call("Notify",&send("warning")).await?;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(std::fs::read_to_string(&output)?,format!("{warning},"),
        "quiet mode must override the category exec hook");
    let _:()=ctl.call("SetMode",&("default",)).await?;
    let again:u32=dest.call("Notify",&send("warning")).await?;
    tokio::time::timeout(std::time::Duration::from_secs(3),async{
        loop {
            if std::fs::read_to_string(&output).unwrap_or_default()
                ==format!("{warning},{again},"){break;}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await?;
    Ok(())
}

#[tokio::test]
async fn category_sound_command_routing_and_quiet_mode_preserve_notifications()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // Mako implements sounds via on-notify=exec, not a built-in audio
    // transport. A local file-backed command proves category selection and
    // quiet-mode suppression without depending on a working sound device.
    let dir=tempfile::tempdir()?;
    let marker=dir.path().join("sound-command-events");
    let config=Config::parse(&format!(
        "[category=success]\nbackground-color=#305747\non-notify=exec printf 'success:%s\\n' \"$id\" >> '{}'\n\
         [category=failure]\nbackground-color=#684148\non-notify=exec printf 'failure:%s\\n' \"$id\" >> '{}'\n\
         [category=warning]\nbackground-color=#715536\non-notify=exec printf 'warning:%s\\n' \"$id\" >> '{}'\n\
         [mode=quiet]\non-notify=none\n",
        marker.display(),marker.display(),marker.display(),
    ))?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let category_color=[
        ("success","#305747"),("failure","#684148"),("warning","#715536")
    ];
    let mut expected=Vec::new();
    for (category,color) in category_color {
        let hints:HashMap<String,OwnedValue>=[(
            "category".to_owned(),
            OwnedValue::try_from(zbus::zvariant::Value::from(category))?,
        )].into();
        let id:u32=desktop.call("Notify",&(
            "sound-router",0u32,"","Category","",
            Vec::<String>::new(),hints,-1i32,
        )).await?;
        expected.push(format!("{category}:{id}"));
        let note=runtime.store.lock().unwrap().active().iter()
            .find(|note|note.id==id).cloned().ok_or("missing routed note")?;
        let style=posthorn::desktop::style_for_note(
            &runtime,&note,&["default".to_owned()]);
        assert_eq!(style["background-color"],color);
    }
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        loop {
            if std::fs::read_to_string(&marker).unwrap_or_default()
                .lines().count()==3 {break;}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }).await?;
    expected.sort();
    let text=std::fs::read_to_string(&marker)?;
    let mut observed=text.lines().map(str::to_owned).collect::<Vec<_>>();
    observed.sort();
    assert_eq!(observed,expected,
        "each category must select its own sound hook once");
    let _:()=control.call("SetMode",&("quiet",)).await?;
    for (category,color) in category_color {
        let hints:HashMap<String,OwnedValue>=[(
            "category".to_owned(),
            OwnedValue::try_from(zbus::zvariant::Value::from(category))?,
        )].into();
        let id:u32=desktop.call("Notify",&(
            "sound-router",0u32,"","Quiet category","",
            Vec::<String>::new(),hints,-1i32,
        )).await?;
        let note=runtime.store.lock().unwrap().active().iter()
            .find(|note|note.id==id).cloned().ok_or("missing quiet note")?;
        let style=posthorn::desktop::style_for_note(
            &runtime,&note,&["quiet".to_owned()]);
        assert_eq!(style["background-color"],color,
            "quiet mode must preserve category presentation while muting hooks");
    }
    tokio::time::sleep(std::time::Duration::from_millis(160)).await;
    assert_eq!(std::fs::read_to_string(&marker)?,text,
        "quiet mode must suppress all category sound commands");
    assert_eq!(runtime.store.lock().unwrap().active().len(),6,
        "muting notification hooks must not discard the notifications");
    Ok(())
}

#[tokio::test]
async fn stack_tag_replacement_consumes_mako_notification_counter_over_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // Mako v1.11 create_notification runs before stack-tag lookup even
    // when the replacement ultimately returns an existing notification ID.
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let tagged:HashMap<String,OwnedValue>=[(
        "x-dunst-stack-tag".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("progress"))?
    )].into();
    let ordinary:HashMap<String,OwnedValue>=HashMap::new();
    let notify=|replace:u32,title:&str,hints:HashMap<String,OwnedValue>|(
        "tag-counter-fixture",replace,"",title.to_string(),"",
        Vec::<String>::new(),hints,-1i32
    );
    let first:u32=desktop.call("Notify",&notify(0,"Initial",tagged.clone())).await?;
    let same:u32=desktop.call("Notify",&notify(0,"First update",tagged.clone())).await?;
    assert_eq!(same,first);
    let same:u32=desktop.call("Notify",&notify(0,"Second update",tagged)).await?;
    assert_eq!(same,first);
    let next:u32=desktop.call("Notify",&notify(0,"Ordinary",ordinary.clone())).await?;
    assert_eq!(next,first+3,
        "tag replacements must advance the next D-Bus notification ID");
    let reset:u32=desktop.call("Notify",&notify(next,"Direct update",ordinary.clone())).await?;
    assert_eq!(reset,next);
    let next_after_direct:u32=desktop.call("Notify",&notify(0,"After direct",ordinary)).await?;
    assert_eq!(next_after_direct,next+1,
        "a direct ID replacement must not advance the counter");
    Ok(())
}

#[tokio::test]
async fn history_restore_reapplies_category_anchor_without_timer_over_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::parse(
        "default-timeout=4000\n[category=warning]\nanchor=bottom-left\n\
         [anchor=bottom-left]\nbackground-color=#416276\n"
    )?).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let hints:HashMap<String,OwnedValue>=[(
        "category".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("warning"))?
    )].into();
    let id:u32=desktop.call("Notify",&(
        "restore-anchor",0u32,"","Archive","",Vec::<String>::new(),
        hints,-1i32
    )).await?;
    let options:HashMap<String,OwnedValue>=[(
        "id".into(),OwnedValue::from(id)
    )].into();
    let _:()=control.call("DismissNotifications",&(options,)).await?;
    let historical=runtime.store.lock().unwrap().history().front()
        .ok_or("notification not archived")?.clone();
    assert!(!historical.surface_associated);
    assert!(historical.surface_anchor.is_none());
    let _:()=control.call("RestoreNotification",&()).await?;
    let restored=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).ok_or("notification not restored")?.clone();
    assert_eq!(restored.surface_anchor.as_deref(),Some("bottom-left"));
    let style=posthorn::desktop::style_for_note_index(
        &runtime,&restored,&["default".into()],restored.group_index);
    assert_eq!(style.get("background-color").map(String::as_str),
        Some("#416276"));
    assert!(runtime.next_deadline()?.is_none());
    Ok(())
}

#[tokio::test]
async fn direct_replacement_uses_prior_category_surface_anchor_over_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::parse(
        "default-timeout=4000\n[category=warning]\nanchor=bottom-left\n\
         [anchor=bottom-left]\ndefault-timeout=1000\n"
    )?).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let warning:HashMap<String,OwnedValue>=[(
        "category".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("warning"))?
    )].into();
    let id:u32=desktop.call("Notify",&(
        "anchor-parity",0u32,"","Warning","",Vec::<String>::new(),
        warning,-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().surface_anchor.as_deref(),
        Some("bottom-left"));
    let first_remaining=runtime.next_deadline()?
        .ok_or("missing original timeout")?
        .saturating_duration_since(std::time::Instant::now());
    assert!(first_remaining>Duration::from_secs(3));
    let replaced:u32=desktop.call("Notify",&(
        "anchor-parity",id,"","Normal","",Vec::<String>::new(),
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_eq!(replaced,id);
    let remaining=runtime.next_deadline()?
        .ok_or("missing replacement timeout")?
        .saturating_duration_since(std::time::Instant::now());
    assert!(remaining>Duration::from_millis(200)
        && remaining<=Duration::from_secs(1),
        "replacement should match old bottom-left surface: {remaining:?}");
    assert_eq!(runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).unwrap().surface_anchor.as_deref(),
        Some("bottom-left"));
    Ok(())
}

#[tokio::test]
async fn new_notifications_do_not_inherit_surface_selectors_over_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::parse(
        "anchor=bottom-left\ndefault-timeout=4000\n\
         [anchor=bottom-left]\ndefault-timeout=2000\n"
    )?).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let tagged:HashMap<String,OwnedValue>=[(
        "x-dunst-stack-tag".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("one"))?
    )].into();
    let first:u32=desktop.call("Notify",&(
        "surface-fixture",0u32,"","Initial","",Vec::<String>::new(),
        tagged.clone(),-1i32
    )).await?;
    let first_deadline=runtime.next_deadline()?.ok_or("missing first timeout")?;
    assert!(first_deadline.saturating_duration_since(std::time::Instant::now())
        >Duration::from_secs(3));
    assert!(runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==first).unwrap().surface_associated);

    let direct:u32=desktop.call("Notify",&(
        "surface-fixture",first,"","Direct","",Vec::<String>::new(),
        tagged.clone(),-1i32
    )).await?;
    assert_eq!(direct,first);
    let direct_left=runtime.next_deadline()?.ok_or("missing replacement timeout")?
        .saturating_duration_since(std::time::Instant::now());
    assert!(direct_left>Duration::from_secs(1)&&direct_left<=Duration::from_secs(2),
        "direct-ID replacement should inherit anchor: {direct_left:?}");

    let tag:u32=desktop.call("Notify",&(
        "surface-fixture",0u32,"","Tag","",Vec::<String>::new(),
        tagged,-1i32
    )).await?;
    assert_eq!(tag,first);
    let tag_left=runtime.next_deadline()?.ok_or("missing tag timeout")?
        .saturating_duration_since(std::time::Instant::now());
    assert!(tag_left>Duration::from_secs(3)&&tag_left<=Duration::from_secs(4),
        "tag replacement must start with NULL surface: {tag_left:?}");
    Ok(())
}

#[tokio::test]
async fn direct_id_replacement_preserves_pre_regroup_group_index_timeout_over_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // The daemon's private development bus is used exclusively here.
    let config=Config::parse(
        "group-by=app-name\ndefault-timeout=0\n\
         [group-index=1]\ndefault-timeout=5000\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let options=HashMap::<String,OwnedValue>::new();
    let first:u32=desktop.call("Notify",&(
        "same-app",0u32,"","First","",Vec::<String>::new(),options.clone(),-1i32
    )).await?;
    let _:u32=desktop.call("Notify",&(
        "same-app",0u32,"","Second","",Vec::<String>::new(),options.clone(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==first).unwrap().group_index,1);
    assert_eq!(runtime.next_deadline()?,None);
    let replaced:u32=desktop.call("Notify",&(
        "same-app",first,"","Replaced","",Vec::<String>::new(),options,-1i32
    )).await?;
    assert_eq!(replaced,first);
    let deadline=runtime.next_deadline()?;
    assert!(deadline.is_some(),
        "Mako must arm [group-index=1] timeout before replacement regrouping");
    let remaining=deadline.unwrap().saturating_duration_since(std::time::Instant::now());
    assert!(remaining>Duration::from_secs(1)&&remaining<=Duration::from_secs(5),
        "unexpected direct replacement timeout: {remaining:?}");
    Ok(())
}

#[tokio::test]
async fn on_notify_binding_is_selected_before_grouping_over_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // Mako first applies criteria and stores the binding, then groups,
    // then executes the preselected binding. Grouped styles therefore do
    // not dismiss a *new* member, but do apply on an explicit replacement
    // whose pre-existing group index is retained by reset_notification.
    let (conn,runtime)=open_private_service(Config::parse(
        "group-by=app-name\n[grouped]\non-notify=dismiss\n"
    )?).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let options=HashMap::<String,OwnedValue>::new();
    let first:u32=desktop.call("Notify",&(
        "grouped-hook",0u32,"","First","",Vec::<String>::new(),
        options.clone(),-1i32
    )).await?;
    let second:u32=desktop.call("Notify",&(
        "grouped-hook",0u32,"","Second","",Vec::<String>::new(),
        options.clone(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),2,
        "newly grouped messages cannot retroactively select [grouped] on-notify");
    assert_eq!(runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==first).unwrap().group_index,1);
    let replaced:u32=desktop.call("Notify",&(
        "grouped-hook",first,"","Direct replacement","",
        Vec::<String>::new(),options.clone(),-1i32
    )).await?;
    assert_eq!(replaced,first);
    assert!(runtime.store.lock().unwrap().active().iter()
        .all(|n|n.id!=first),
        "existing ID was grouped at style selection and should dismiss");
    assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    assert_eq!(runtime.store.lock().unwrap().active().front().unwrap().id,second);
    Ok(())
}

#[tokio::test]
async fn id_and_tag_replacements_have_distinct_group_lifecycle_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let config=Config::parse(
        "group-by=none\n[category=warning]\ngroup-by=app-name\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let ordinary:HashMap<String,OwnedValue>=HashMap::new();
    let warning:HashMap<String,OwnedValue>=[
        ("category".into(),
         OwnedValue::try_from(zbus::zvariant::Value::from("warning"))?)
    ].into();
    let direct:u32=desktop.call("Notify",&(
        "direct-app",0u32,"","Old","",Vec::<String>::new(),
        ordinary.clone(),-1i32
    )).await?;
    let _:u32=desktop.call("Notify",&(
        "direct-app",0u32,"","Grouping warning","",Vec::<String>::new(),
        warning.clone(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==direct).unwrap().group_index,1);
    let before_explicit=runtime.store.lock().unwrap().active_instance(direct)
        .ok_or("missing initial direct Notify lifetime")?;
    let replaced:u32=desktop.call("Notify",&(
        "direct-app",direct,"","ID reset","",Vec::<String>::new(),
        ordinary.clone(),-1i32
    )).await?;
    assert_eq!(replaced,direct);
    assert_ne!(runtime.store.lock().unwrap().active_instance(direct),
        Some(before_explicit),"explicit D-Bus replacement must invalidate stale clicks");
    let direct_note=runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==direct).unwrap().clone();
    assert_eq!(direct_note.group_index,1,
        "reset_notification on an existing ID retains its group index");
    assert_eq!(direct_note.summary,"ID reset");

    let tagged:HashMap<String,OwnedValue>=[(
        "x-dunst-stack-tag".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("progress"))?
    )].into();
    let old:u32=desktop.call("Notify",&(
        "tag-app",0u32,"","Old tag","",Vec::<String>::new(),
        tagged.clone(),-1i32
    )).await?;
    let _:u32=desktop.call("Notify",&(
        "tag-app",0u32,"","Grouping warning","",Vec::<String>::new(),
        warning,-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==old).unwrap().group_index,1);
    let before_tag=runtime.store.lock().unwrap().active_instance(old)
        .ok_or("missing initial stack-tag lifetime")?;
    let tag_replacement:u32=desktop.call("Notify",&(
        "tag-app",0u32,"","New tag object","",Vec::<String>::new(),
        tagged,-1i32
    )).await?;
    assert_eq!(tag_replacement,old);
    assert_ne!(runtime.store.lock().unwrap().active_instance(old),Some(before_tag),
        "tag replacement through D-Bus must invalidate stale pointer events");
    let tag_note=runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==old).unwrap().clone();
    assert_eq!((tag_note.group_index,tag_note.group_count),(-1,1),
        "tag replacement is a fresh notification occupying the old slot");
    assert_eq!(tag_note.summary,"New tag object");
    let listed:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    assert_eq!(listed.iter().filter(|m|
        m.get("id").and_then(|v|u32::try_from(v).ok())==Some(old)).count(),1);
    Ok(())
}

#[tokio::test]
async fn grouped_history_follows_arrival_and_dismissal_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse("group-by=app-name\n[grouped]\nhistory=0\n")?;
    let (conn,_runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let first:u32=desktop.call("Notify",&(
        "sender-app",0u32,"","First","",Vec::<String>::new(),
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let second:u32=desktop.call("Notify",&(
        "sender-app",0u32,"","Second","",Vec::<String>::new(),
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let opts:HashMap<String,OwnedValue>=[("id".into(),OwnedValue::from(second))].into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let history:Vec<HashMap<String,OwnedValue>>=ctl.call("ListHistory",&()).await?;
    assert!(history.is_empty(),
        "grouped notification must honor its scoped history=0");
    let opts:HashMap<String,OwnedValue>=[("id".into(),OwnedValue::from(first))].into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let history:Vec<HashMap<String,OwnedValue>>=ctl.call("ListHistory",&()).await?;
    assert_eq!(history.len(),1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?,first);
    Ok(())
}

#[tokio::test]
async fn mode_reapplication_regroups_live_dbus_notification_list()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::desktop::visible_members_indexed;
    let config=Config::parse(
        "group-by=none\n[mode=quiet]\ngroup-by=app-name\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let send=|app:&str|(app.to_owned(),0u32,"","Notify","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32);
    let older:u32=desktop.call("Notify",&send("same")).await?;
    let other:u32=desktop.call("Notify",&send("different")).await?;
    let newer:u32=desktop.call("Notify",&send("same")).await?;
    let _:()=ctl.call("SetMode",&("quiet",)).await?;
    let listed:Vec<HashMap<String,OwnedValue>>=ctl.call("ListNotifications",&()).await?;
    assert_eq!(listed.iter().map(|m|u32::try_from(m.get("id").unwrap()).unwrap())
        .collect::<Vec<_>>(),vec![newer,older,other]);
    assert_eq!(visible_members_indexed(&runtime)?.iter().map(|m|
        (m.notification.id,m.group_count)).collect::<Vec<_>>(),
        vec![(newer,2),(other,1)]);
    let _:()=ctl.call("SetMode",&("default",)).await?;
    assert_eq!(visible_members_indexed(&runtime)?.len(),3,
        "unsetting the grouped mode restores independent visibility");
    Ok(())
}

#[tokio::test]
async fn mode_dbus_reapplies_history_of_existing_notifications()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let config=Config::parse(
        "history=1\n[mode=quiet]\nhistory=0\n"
    )?;
    let (conn,_runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let new_notification=||(
        "history-mode-fixture",0u32,"","Still active","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    );
    let first:u32=desktop.call("Notify",&new_notification()).await?;
    let _:()=ctl.call("SetMode",&("quiet",)).await?;
    let opts:HashMap<String,OwnedValue>=[("id".into(),OwnedValue::from(first))]
        .into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let history:Vec<HashMap<String,OwnedValue>>=ctl.call("ListHistory",&()).await?;
    assert!(history.is_empty(),"mode change must affect active note history");

    let second:u32=desktop.call("Notify",&new_notification()).await?;
    let _:()=ctl.call("SetMode",&("default",)).await?;
    let opts:HashMap<String,OwnedValue>=[("id".into(),OwnedValue::from(second))]
        .into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let history:Vec<HashMap<String,OwnedValue>>=ctl.call("ListHistory",&()).await?;
    assert_eq!(history.len(),1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?,second);
    Ok(())
}

#[tokio::test]
async fn reload_rebuilds_live_grouping_and_preserves_timer_deadline()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::desktop::visible_members_indexed;
    use posthorn::service::open_private_service_from_path;
    let dir=tempfile::tempdir()?;
    let path=dir.path().join("config");
    std::fs::write(&path,"group-by=none\ndefault-timeout=15000\n")?;
    let (conn,runtime)=open_private_service_from_path(&path).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let send=|app:&str|(app.to_owned(),0u32,"","Notice","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32);
    let first:u32=desktop.call("Notify",&send("same")).await?;
    let other:u32=desktop.call("Notify",&send("other")).await?;
    let last:u32=desktop.call("Notify",&send("same")).await?;
    let before=runtime.next_deadline()?;
    assert!(before.is_some());
    std::fs::write(&path,"group-by=app-name\ndefault-timeout=300\n")?;
    let _:()=ctl.call("Reload",&()).await?;
    assert_eq!(runtime.next_deadline()?,before,
        "Mako reapply_config must not restart existing notification timers");
    let listed:Vec<HashMap<String,OwnedValue>>=ctl.call("ListNotifications",&()).await?;
    assert_eq!(listed.iter().map(|n|u32::try_from(n.get("id").unwrap()).unwrap())
        .collect::<Vec<_>>(),vec![last,first,other]);
    assert_eq!(visible_members_indexed(&runtime)?.iter()
        .map(|member|(member.notification.id,member.group_count))
        .collect::<Vec<_>>(),vec![(last,2),(other,1)]);
    Ok(())
}

#[tokio::test]
async fn private_dbus_reload_defers_history_retention_pruning_until_new_close()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // Pinned Mako v1.11 reloads max-history without walking existing
    // history. The next close that archives an entry enforces the cap.
    use posthorn::service::open_private_service_from_path;
    let dir=tempfile::tempdir()?;
    let path=dir.path().join("config");
    std::fs::write(&path,"max-history=3\n")?;
    let (conn,_runtime)=open_private_service_from_path(&path).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    for num in 0..3 {
        let id:u32=desktop.call("Notify",&(
            "history-cap-fixture",0u32,"",format!("Before {num}"),"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
        )).await?;
        let options:HashMap<String,OwnedValue>=[
            ("id".into(),OwnedValue::from(id))
        ].into();
        let _:()=control.call("DismissNotifications",&(options,)).await?;
    }
    let before:Vec<HashMap<String,OwnedValue>>=
        control.call("ListHistory",&()).await?;
    assert_eq!(before.len(),3);

    std::fs::write(&path,"max-history=1\n")?;
    let _:()=control.call("Reload",&()).await?;
    let preserved:Vec<HashMap<String,OwnedValue>>=
        control.call("ListHistory",&()).await?;
    assert_eq!(preserved.len(),3,
        "Reload must not prematurely prune the existing Mako history");

    let id:u32=desktop.call("Notify",&(
        "history-cap-fixture",0u32,"","After shrink","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let options:HashMap<String,OwnedValue>=[
        ("id".into(),OwnedValue::from(id))
    ].into();
    let _:()=control.call("DismissNotifications",&(options,)).await?;
    let shrunk:Vec<HashMap<String,OwnedValue>>=
        control.call("ListHistory",&()).await?;
    assert_eq!(shrunk.len(),1);
    assert_eq!(u32::try_from(shrunk[0].get("id").ok_or("no history ID")?)?,id);

    std::fs::write(&path,"max-history=0\n")?;
    let _:()=control.call("Reload",&()).await?;
    let remaining:Vec<HashMap<String,OwnedValue>>=
        control.call("ListHistory",&()).await?;
    assert_eq!(remaining.len(),1);
    let later:u32=desktop.call("Notify",&(
        "history-cap-fixture",0u32,"","Not archived","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let options:HashMap<String,OwnedValue>=[
        ("id".into(),OwnedValue::from(later))
    ].into();
    let _:()=control.call("DismissNotifications",&(options,)).await?;
    let still:Vec<HashMap<String,OwnedValue>>=
        control.call("ListHistory",&()).await?;
    assert_eq!(still.len(),1,
        "max-history=0 must preserve old history and skip new archive");
    assert_eq!(u32::try_from(still[0].get("id").ok_or("no history ID")?)?,id);
    Ok(())
}

#[tokio::test]
async fn reload_dbus_updates_active_history_without_rearming_expiry()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::service::open_private_service_from_path;
    let dir=tempfile::tempdir()?;
    let path=dir.path().join("config");
    std::fs::write(&path,"history=0\ndefault-timeout=3000\n")?;
    let (conn,runtime)=open_private_service_from_path(&path).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let id:u32=desktop.call("Notify",&(
        "reload-history-fixture",0u32,"","Active notification","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let before=runtime.next_deadline()?;
    assert!(before.is_some());
    std::fs::write(&path,"history=1\ndefault-timeout=500\n")?;
    let _:()=ctl.call("Reload",&()).await?;
    assert_eq!(runtime.next_deadline()?,before,
        "Mako does not reschedule existing timeouts on reload");
    let opts:HashMap<String,OwnedValue>=[("id".into(),OwnedValue::from(id))].into();
    let _:()=ctl.call("DismissNotifications",&(opts,)).await?;
    let history:Vec<HashMap<String,OwnedValue>>=ctl.call("ListHistory",&()).await?;
    assert_eq!(history.len(),1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?,id);
    Ok(())
}

#[tokio::test]
async fn control_reload_swaps_styles_and_preserves_last_good_file()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::service::open_private_service_from_path;
    let dir=tempfile::tempdir()?;
    let path=dir.path().join("config");
    std::fs::write(&path,"background-color=#305747\nmax-history=5\n")?;
    let (conn,runtime)=open_private_service_from_path(&path).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let initial=runtime.config();
    assert_eq!(initial.global["background-color"],"#305747");
    std::fs::write(&path,"background-color=#nothex\n")?;
    let failed:Result<(),zbus::Error>=ctl.call("Reload",&()).await;
    match failed {
        Err(zbus::Error::MethodError(name,message,_))=>{
            assert_eq!(name.as_str(),"fr.emersion.Mako.InvalidConfig");
            assert_eq!(message.as_deref(),Some("Unable to parse configuration file"));
        }
        other=>panic!("invalid reload used the wrong D-Bus error: {other:?}"),
    }
    assert_eq!(runtime.config().global["background-color"],"#305747");
    assert_eq!(runtime.revision(),0);
    std::fs::write(&path,"background-color=#684148\nmax-history=2\n")?;
    let result:()=ctl.call("Reload",&()).await?;
    assert_eq!(result,());
    assert_eq!(runtime.config().global["background-color"],"#684148");
    assert_eq!(runtime.revision(),1);
    assert_eq!(initial.global["background-color"],"#305747",
        "old snapshots remain immutable after reload");
    Ok(())
}

#[tokio::test]
async fn deeply_included_config_reload_preserves_last_good_state_on_errors()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::service::open_private_service_from_path;
    let dir=tempfile::tempdir()?;
    let file=|i:usize|dir.path().join(format!("mako-included-{i:02}.conf"));
    for i in 0..20 {
        let content=format!("include={}\n",file(i+1).display());
        std::fs::write(file(i),content)?;
    }
    std::fs::write(file(20),"background-color=#305747\n")?;
    let (conn,runtime)=open_private_service_from_path(&file(0)).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    assert_eq!(runtime.config().global["background-color"],"#305747");

    std::fs::write(file(20),"background-color=#684148\n")?;
    let _:()=ctl.call("Reload",&()).await?;
    assert_eq!(runtime.config().global["background-color"],"#684148");
    assert_eq!(runtime.revision(),1);

    std::fs::write(file(20),"background-color=#invalid\n")?;
    let rejected:zbus::Result<()>=ctl.call("Reload",&()).await;
    assert!(rejected.is_err(),"invalid deep include was accepted");
    assert_eq!(runtime.config().global["background-color"],"#684148");
    assert_eq!(runtime.revision(),1);

    std::fs::write(file(20),format!("include={}\n",file(0).display()))?;
    let cyclic:zbus::Result<()>=ctl.call("Reload",&()).await;
    assert!(cyclic.is_err(),"cyclic deep include was accepted");
    assert_eq!(runtime.config().global["background-color"],"#684148");
    assert_eq!(runtime.revision(),1);
    Ok(())
}

#[tokio::test]
async fn invalid_known_hint_variant_does_not_replace_notification()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let id:u32=desktop.call("Notify",&(
        "malformed-hint-test",0u32,"","Original","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let bad:HashMap<String,OwnedValue>=[
        ("category".into(),OwnedValue::from(41i32))
    ].into();
    let result:zbus::Result<u32>=desktop.call("Notify",&(
        "malformed-hint-test",id,"","Must not replace","",
        Vec::<String>::new(),bad,-1i32
    )).await;
    match result {
        Err(zbus::Error::MethodError(name,_,_))=>
            assert_eq!(name.as_str(),"org.freedesktop.DBus.Error.InvalidArgs"),
        other=>panic!("unexpected wrong-hint result: {other:?}"),
    }
    let store=runtime.store.lock().unwrap();
    assert_eq!(store.active().len(),1);
    assert_eq!(store.active()[0].id,id);
    assert_eq!(store.active()[0].summary,"Original");
    Ok(())
}

#[tokio::test]
async fn absent_urgency_is_wire_unknown_and_not_selector_normal()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let config=Config::parse(
        "width=180\n[urgency=normal]\nwidth=375\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let dest=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let unknown:u32=dest.call("Notify",&(
        "urgency-missing",0u32,"","No hint","",Vec::<String>::new(),
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    let normal:u32=dest.call("Notify",&(
        "urgency-normal",0u32,"","Normal","",Vec::<String>::new(),
        [("urgency".into(),OwnedValue::from(1u8))]
            .into_iter().collect::<HashMap<String,OwnedValue>>(),-1i32
    )).await?;
    let notes=runtime.store.lock().unwrap();
    let unknown_note=notes.active().iter().find(|n|n.id==unknown).unwrap().clone();
    let normal_note=notes.active().iter().find(|n|n.id==normal).unwrap().clone();
    drop(notes);
    let modes=["default".into()];
    assert_eq!(posthorn::desktop::style_for_note(&runtime,&unknown_note,&modes)["width"],"180");
    assert_eq!(posthorn::desktop::style_for_note(&runtime,&normal_note,&modes)["width"],"375");
    let listed:Vec<HashMap<String,OwnedValue>>=ctl.call("ListNotifications",&()).await?;
    let unknown_record=listed.iter().find(|record|
        record.get("id").and_then(|x|u32::try_from(x).ok())==Some(unknown)
    ).ok_or("unknown urgency record missing")?;
    assert_eq!(u8::try_from(unknown_record.get("urgency").unwrap())?,255);
    Ok(())
}

#[tokio::test]
async fn priority_sort_matches_mako_bounded_enum_search_over_private_dbus()
    ->Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::parse(
        "sort=+priority\nsort=+time\ngroup-by=none\n"
    )?).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let mut ids=Vec::new();
    for urgency in [3i32,4i32,-3i32]{
        let hints:HashMap<String,OwnedValue>=[
            ("urgency".into(),OwnedValue::from(urgency))
        ].into();
        let id:u32=desktop.call("Notify",&(
            "priority-fixture",0u32,"","Raw urgency","",
            Vec::<String>::new(),hints,-1i32
        )).await?;
        ids.push(id);
    }
    let records:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    let order:Vec<u32>=records.iter()
        .map(|record|u32::try_from(record.get("id").unwrap()).unwrap())
        .collect();
    assert_eq!(order,vec![ids[2],ids[1],ids[0]],
        "Mako's bounded enum insertion differs from generic signed sorting");
    assert_eq!(runtime.store.lock().unwrap().active().len(),3);
    Ok(())
}

#[tokio::test]
async fn chromium_urgency_variants_are_preserved_over_real_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::notification::Urgency;
    let (conn,runtime)=open_private_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    for (value,expected) in [
        (OwnedValue::from(2u32),Urgency::Critical),
        (OwnedValue::from(0i32),Urgency::Low),
        (OwnedValue::from(2i32),Urgency::Critical),
        (OwnedValue::from(0u8),Urgency::Low),
        (OwnedValue::from(-3i32),Urgency::Other(-3)),
        (OwnedValue::from(3u32),Urgency::Other(3)),
        (OwnedValue::from(255u8),Urgency::Other(255)),
    ] {
        let hints:HashMap<String,OwnedValue>=[("urgency".into(),value)].into();
        let id:u32=desktop.call("Notify",&(
            "urgency-fixture",0u32,"","Hint","",Vec::<String>::new(),
            hints,-1i32
        )).await?;
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==id).unwrap().urgency,expected);
        let all:Vec<HashMap<String,OwnedValue>>=
            ctl.call("ListNotifications",&()).await?;
        let record=all.iter().find(|item|
            item.get("id").and_then(|v|u32::try_from(v).ok())==Some(id)
        ).ok_or("missing wire notification")?;
        let urgency=u8::try_from(record.get("urgency").unwrap())?;
        assert_eq!(urgency,match expected{
            Urgency::Unknown=>255,
            Urgency::Low=>0,Urgency::Normal=>1,Urgency::Critical=>2,
            Urgency::Other(value)=>value as u8,
        });
    }
    Ok(())
}

#[tokio::test]
async fn raw_signed_progress_hint_preserved_across_replacements_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::default()).await?;
    let dest=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let send=|replaces_id:u32,value:Option<i32>|(
        "progress-wire",replaces_id,"","Status","",Vec::<String>::new(),
        value.map(|v|[("value".into(),OwnedValue::from(v))]
            .into_iter().collect::<HashMap<String,OwnedValue>>())
            .unwrap_or_default(),-1i32
    );
    let id:u32=dest.call("Notify",&send(0,Some(170))).await?;
    assert_eq!(runtime.store.lock().unwrap().active()[0].progress,170);
    let replaced:u32=dest.call("Notify",&send(id,Some(-10))).await?;
    assert_eq!(replaced,id);
    assert_eq!(runtime.store.lock().unwrap().active()[0].progress,-10);
    let replaced:u32=dest.call("Notify",&send(id,None)).await?;
    assert_eq!(replaced,id);
    assert_eq!(runtime.store.lock().unwrap().active()[0].progress,-1);
    Ok(())
}

#[tokio::test]
async fn private_dbus_progress_value_has_exact_mako_integer_pixel_boundary()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::render::render_notification_group_scaled;
    let config=Config::parse(
        "width=101\nheight=70\nborder-size=0\npadding=0\n         progress-color=source #FF0000FF\nbackground-color=#0000FFFF\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let dest=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let hints:HashMap<String,OwnedValue>=[("value".into(),OwnedValue::from(33i32))].into();
    let id:u32=dest.call("Notify",&(
        "progress-boundary",0u32,"","Status","",Vec::<String>::new(),
        hints,-1i32
    )).await?;
    let note=runtime.store.lock().unwrap().active().iter()
        .find(|n|n.id==id).cloned().ok_or("missing notification")?;
    assert_eq!(note.progress,33);
    let style=runtime.config().global.clone();
    for scale in [1u32,2] {
        let frame=render_notification_group_scaled(&note,&style,0,scale)?;
        let y=(frame.height as usize)-2*scale as usize;
        let color=|x:usize| {
            let at=y*frame.stride as usize+x*4;
            frame.pixels[at..at+4].to_vec()
        };
        assert_eq!(color(33*scale as usize-1),[0,0,255,255],
            "last Mako integer-width progress pixel at {scale}x is solid red");
        assert_eq!(color(33*scale as usize),[255,0,0,255],
            "first unfilled pixel at {scale}x must stay solid blue");
    }
    Ok(())
}

#[tokio::test]
async fn dbus_inline_image_and_progress_reach_rendered_pixels()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::desktop::snapshot_frame;
    let config=Config::parse(
        "width=160\nheight=90\nborder-size=0\npadding=4\n         progress-color=#00FF00\nbackground-color=#000000\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let dest=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let data=(8i32,8i32,32i32,true,8i32,4i32,
        [255u8,0,0,255].repeat(64));
    let inline=OwnedValue::try_from(zbus::zvariant::Value::from(data))?;
    let hints:HashMap<String,OwnedValue>=[
        ("image-data".into(),inline),
        ("value".into(),OwnedValue::from(50i32))
    ].into();
    let id:u32=dest.call("Notify",&(
        "wire-image-test",0u32,"","Progress","Running",
        Vec::<String>::new(),hints,-1i32
    )).await?;
    let store=runtime.store.lock().unwrap();
    let note=store.active().iter().find(|n|n.id==id).unwrap();
    assert_eq!(note.progress,50);
    assert!(note.image_data.is_some());
    drop(store);
    let frame=snapshot_frame(&runtime)?.unwrap();
    // Search the rendered raster rather than assuming a particular Pango
    // text height or icon offset after Mako's directional margins.
    assert!(frame.pixels.chunks_exact(4).any(|p|p==[0,0,255,255]),
        "inline red icon pixels must actually be visible");
    assert!(frame.pixels.chunks_exact(4).any(|p|p==[0,255,0,255]),
        "the D-Bus value hint must render a green progress fill");
    Ok(())
}

#[tokio::test]
async fn actions_false_suppresses_action_signals_but_click_still_dismisses()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use futures_util::StreamExt;
    use posthorn::input::dispatch_click;
    use posthorn::wayland::PointerClick;
    let config=Config::parse(
        "width=160\nheight=90\nmargin=6\n[category=warning]\nactions=FALSE\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let mut signals=desktop.receive_all_signals().await?;
    let hints:HashMap<String,OwnedValue>=[(
        "category".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("warning"))?,
    )].into();
    let id:u32=desktop.call("Notify",&(
        "action-style-fixture",0u32,"","Visible","",
        vec!["default".to_string(),"Open".to_string()],
        hints,-1i32
    )).await?;
    let _:()=ctl.call("InvokeAction",&(id,"default")).await?;
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(120),signals.next()
    ).await.is_err(),"actions=FALSE must suppress control ActionInvoked");
    assert_eq!(runtime.store.lock().unwrap().active().len(),1,
        "Mako control InvokeAction must not dismiss the notification");
    dispatch_click(&conn,&runtime,PointerClick{
        x:30.0,y:20.0,button:272,serial:15,
    }).await?;
    let closed=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("expected click close signal")?;
    assert_eq!(closed.header().member().unwrap().as_str(),"NotificationClosed");
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(120),signals.next()
    ).await.is_err(),"actions=FALSE click must not emit a later action signal");
    assert!(runtime.store.lock().unwrap().active().is_empty());
    Ok(())
}

#[tokio::test]
async fn control_action_signals_follow_mode_styles_for_existing_notification()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use futures_util::StreamExt;
    let config=Config::parse(
        "[category=warning]\nactions=0\n[mode=allow-action]\nactions=1\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let mut signals=desktop.receive_all_signals().await?;
    let hints:HashMap<String,OwnedValue>=[(
        "category".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("warning"))?,
    )].into();
    let id:u32=desktop.call("Notify",&(
        "mode-actions",0u32,"","Notice","",
        vec!["default".to_string(),"Open".to_string()],hints,-1i32
    )).await?;
    let _:()=ctl.call("InvokeAction",&(id,"default")).await?;
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(100),signals.next()
    ).await.is_err(),"disabled action emitted a signal");
    let _:()=ctl.call("SetModes",&(vec!["allow-action".to_string()],)).await?;
    let _:()=ctl.call("InvokeAction",&(id,"default")).await?;
    let invoked=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("missing action after mode switch")?;
    assert_eq!(invoked.header().member().unwrap().as_str(),"ActionInvoked");
    let (target,key):(u32,String)=invoked.body().deserialize()?;
    assert_eq!((target,key),(id,"default".into()));
    let _:()=ctl.call("SetModes",&(vec!["default".to_string()],)).await?;
    let _:()=ctl.call("InvokeAction",&(id,"default")).await?;
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(100),signals.next()
    ).await.is_err(),"disabling actions again emitted a signal");
    assert_eq!(runtime.store.lock().unwrap().active().len(),1,
        "control InvokeAction must not close a notification");
    Ok(())
}

#[tokio::test]
async fn click_activation_signal_precedes_action_and_close()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use futures_util::StreamExt;
    use posthorn::input::dispatch_click_with_activation;
    use posthorn::wayland::PointerClick;
    let config=Config::parse(
        "width=160\nheight=90\nmargin=6\n         on-button-left=invoke-default-action\n"
    )?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let mut signals=desktop.receive_all_signals().await?;
    let id:u32=desktop.call("Notify",&(
        "token-fixture",0u32,"","Action","Visible",
        vec!["default".to_string(),"Open".to_string()],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    dispatch_click_with_activation(
        &conn,&runtime,PointerClick{x:30.0,y:20.0,button:272,serial:42},
        Some("fixture-compositor-token")
    ).await?;
    let mut names=Vec::new();
    for _ in 0..3 {
        let message=tokio::time::timeout(
            std::time::Duration::from_secs(3),signals.next()
        ).await?.ok_or("expected next notification signal")?;
        let name=message.header().member().ok_or("signal without a member")?
            .as_str().to_string();
        if name=="ActivationToken" {
            let (target,token):(u32,String)=message.body().deserialize()?;
            assert_eq!(target,id);
            assert_eq!(token,"fixture-compositor-token");
        }
        names.push(name);
    }
    assert_eq!(names,vec![
        "ActivationToken","ActionInvoked","NotificationClosed",
    ]);
    assert!(runtime.store.lock().unwrap().active().is_empty());
    Ok(())
}

#[tokio::test]
async fn out_of_range_urgency_does_not_match_normal_style_over_dbus()
    ->Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service(Config::parse(
        "[urgency=normal]\non-notify=dismiss\n"
    )?).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let ctl=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let ordinary:HashMap<String,OwnedValue>=[(
        "urgency".into(),OwnedValue::from(1u8)
    )].into();
    let _:u32=desktop.call("Notify",&(
        "urgency-parity",0u32,"","Normal closes","",
        Vec::<String>::new(),ordinary,-1i32
    )).await?;
    assert!(runtime.store.lock().unwrap().active().is_empty());
    let unusual:HashMap<String,OwnedValue>=[(
        "urgency".into(),OwnedValue::from(3u32)
    )].into();
    let id:u32=desktop.call("Notify",&(
        "urgency-parity",0u32,"","Unknown value stays","",
        Vec::<String>::new(),unusual,-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    let records:Vec<HashMap<String,OwnedValue>>=
        ctl.call("ListNotifications",&()).await?;
    assert_eq!(records.len(),1);
    assert_eq!(u32::try_from(records[0].get("id").unwrap())?,id);
    assert_eq!(u8::try_from(records[0].get("urgency").unwrap())?,3u8);
    Ok(())
}

#[tokio::test]
async fn posix_dotall_summary_criteria_drive_private_dbus_on_notify()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse(
        "[summary~=^Line.One$]\non-notify=dismiss\n"
    )?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let id:u32=desktop.call("Notify",&(
        "multiline-criteria",0u32,"","Line\nOne","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    {
        let store=runtime.store.lock().unwrap();
        assert!(store.active().is_empty(),
            "Mako POSIX REG_EXTENDED without REG_NEWLINE matches newline with dot");
        assert_eq!(store.history().front().map(|note|note.id),Some(id));
    }
    let other:u32=desktop.call("Notify",&(
        "multiline-criteria",0u32,"","Prefix\nLine\nOne","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().front().map(|n|n.id),
        Some(other),"^ must still anchor to the complete notification summary");
    Ok(())
}

#[tokio::test]
async fn posix_literal_backslash_d_criteria_drive_private_dbus_on_notify()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse(
        "[summary~=\\\\d+]\non-notify=dismiss\n[summary~=\\\\D+]\non-notify=dismiss\n"
    )?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    for summary in ["ddd","DDD"] {
        let id:u32=desktop.call("Notify",&(
            "posix-escaped-letter",0u32,"",summary,"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
        )).await?;
        let store=runtime.store.lock().unwrap();
        assert!(store.active().is_empty(),
            "Mako POSIX ERE treats backslash-d and backslash-D as literal letters");
        assert_eq!(store.history().front().map(|n|n.id),Some(id));
    }
    let id:u32=desktop.call("Notify",&(
        "posix-escaped-letter",0u32,"","123","",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().front().map(|n|n.id),
        Some(id),"Rust's digit-class interpretation must not trigger a Mako rule");
    Ok(())
}

#[tokio::test]
async fn glibc_literal_hex_and_end_anchor_spellings_drive_private_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse(
        "[summary~=^\\\\x41$]\non-notify=dismiss\n[summary~=^foo\\\\z$]\non-notify=dismiss\n"
    )?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    for summary in ["x41", "fooz"] {
        let id:u32=desktop.call("Notify",&(
            "posix-escapes",0u32,"",summary,"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
        )).await?;
        let store=runtime.store.lock().unwrap();
        assert!(store.active().is_empty(),"{summary:?} must trigger Mako matching");
        assert_eq!(store.history().front().map(|n|n.id),Some(id));
    }
    for summary in ["A", "foo"] {
        let id:u32=desktop.call("Notify",&(
            "posix-escapes",0u32,"",summary,"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
        )).await?;
        assert!(runtime.store.lock().unwrap().active().iter().any(|n|n.id==id),
            "Rust-only escape meaning must not dismiss {summary:?}");
    }
    Ok(())
}

#[tokio::test]
async fn posix_bracket_and_escape_literal_rules_apply_to_live_notifications()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse(
        "[summary~=^[\\\\w]+$]\non-notify=dismiss\n[summary~=^\\\\n+$]\non-notify=dismiss\n"
    )?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    for summary in ["www", "nn"] {
        let id:u32=desktop.call("Notify",&(
            "bracket-posix",0u32,"",summary,"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
        )).await?;
        let store=runtime.store.lock().unwrap();
        assert!(store.active().is_empty(),"{summary:?} should match glibc literal");
        assert_eq!(store.history().front().map(|n|n.id),Some(id));
    }
    for summary in ["hello_123", "\n"] {
        let id:u32=desktop.call("Notify",&(
            "bracket-posix",0u32,"",summary,"",
            Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
        )).await?;
        assert!(runtime.store.lock().unwrap().active().iter().any(|n|n.id==id),
            "{summary:?} incorrectly matched a Rust-only escape class");
    }
    Ok(())
}

#[tokio::test]
async fn on_notify_bindings_dismiss_and_invoke_without_replay()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use futures_util::StreamExt;
    let cfg=Config::parse(
        "[category=ephemeral]\non-notify=dismiss\n         [category=interactive]\non-notify=invoke-default-action\n"
    )?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let mut signals=desktop.receive_all_signals().await?;
    let hints:HashMap<String,OwnedValue>=[(
        "category".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("ephemeral"))?,
    )].into();
    let first:u32=desktop.call("Notify",&(
        "fixture",0u32,"","Auto dismiss","",
        Vec::<String>::new(),hints,-1i32,
    )).await?;
    let closed=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("missing automatic dismissal signal")?;
    assert_eq!(closed.header().member().unwrap().as_str(),"NotificationClosed");
    assert_eq!(closed.body().deserialize::<(u32,u32)>()?,(first,2));
    assert!(runtime.store.lock().unwrap().active().is_empty());
    assert_eq!(runtime.store.lock().unwrap().history().len(),1);
    let _:()=control.call("RestoreNotification",&()).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),1,
        "restoring must not replay the on-notify dismissal");

    let hints:HashMap<String,OwnedValue>=[(
        "category".into(),
        OwnedValue::try_from(zbus::zvariant::Value::from("interactive"))?,
    )].into();
    let next:u32=desktop.call("Notify",&(
        "fixture",0u32,"","Auto action","",
        vec!["default".to_string(),"Open".to_string()],hints,-1i32,
    )).await?;
    let action=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("missing automatic action")?;
    assert_eq!(action.header().member().unwrap().as_str(),"ActionInvoked");
    assert_eq!(action.body().deserialize::<(u32,String)>()?,
        (next,"default".into()));
    let closed=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("missing action close")?;
    assert_eq!(closed.header().member().unwrap().as_str(),"NotificationClosed");
    assert_eq!(closed.body().deserialize::<(u32,u32)>()?,(next,2));
    Ok(())
}

#[tokio::test]
async fn grouped_on_notify_exec_requires_preexisting_group_on_direct_replace_not_restore()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let dir=tempfile::tempdir()?;
    let marker=dir.path().join("grouped-hook");
    let config=Config::parse(&format!(
        "group-by=app-name\n         [grouped]\non-notify=exec touch '{}'\n",
        marker.display()
    ))?;
    let (conn,runtime)=open_private_service(config).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let actions=Vec::<String>::new();
    let hints=HashMap::<String,OwnedValue>::new();
    let _:u32=desktop.call("Notify",&(
        "same",0u32,"","First","",actions.clone(),hints.clone(),-1i32
    )).await?;
    assert!(!marker.exists(),"ungrouped first notification must not run group hook");
    let second:u32=desktop.call("Notify",&(
        "same",0u32,"","Second","",actions.clone(),hints.clone(),-1i32
    )).await?;
    // Mako applies criteria before group_notifications. A fresh second
    // notification is still ungrouped while its on-notify binding is chosen.
    // Its post-grouping [grouped] style must not retroactively run exec.
    assert!(!marker.exists(),"newly formed group must not trigger on-notify");
    assert_eq!(runtime.store.lock().unwrap().active()
        .iter().find(|n|n.id==second).unwrap().group_index,0);
    let replaced:u32=desktop.call("Notify",&(
        "same",second,"","Direct replacement","",actions,hints,-1i32
    )).await?;
    assert_eq!(replaced,second);
    // Explicit replaces_id preserves the previous group index during
    // criteria selection, so this time the grouped exec is selected.
    tokio::time::timeout(std::time::Duration::from_secs(3),async{
        while !marker.exists(){
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await?;
    std::fs::remove_file(&marker)?;
    let mut opts=HashMap::new();
    opts.insert("id".to_string(),OwnedValue::from(second));
    let _:()=control.call("DismissNotifications",&(opts,)).await?;
    let _:()=control.call("RestoreNotification",&()).await?;
    assert!(runtime.store.lock().unwrap().active().iter().any(|n|n.id==second));
    assert!(!marker.exists(),"restoring must not launch the grouped hook again");
    Ok(())
}

#[tokio::test]
async fn quiet_mode_silences_category_sound_hooks_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let dir=tempfile::tempdir()?;
    let marker=dir.path().join("success-audio");
    let cfg=Config::parse(&format!(
        "[category=success]\nbackground-color=#305747\n         on-notify=exec touch '{}'\n         [mode=quiet]\non-notify=none\n",
        marker.display()
    ))?;
    let (conn,runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let hints=||->Result<HashMap<String,OwnedValue>,zbus::zvariant::Error>{
        Ok([("category".to_owned(),
            OwnedValue::try_from(zbus::zvariant::Value::from("success"))?)].into())
    };
    *runtime.modes().lock().unwrap()=vec!["quiet".into()];
    let _:u32=desktop.call("Notify",&(
        "audio-fixture",0u32,"","Quiet success","",
        Vec::<String>::new(),hints()?,-1i32
    )).await?;
    assert!(!marker.exists());
    assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    *runtime.modes().lock().unwrap()=vec!["default".into()];
    let _:u32=desktop.call("Notify",&(
        "audio-fixture",0u32,"","Audible success","",
        Vec::<String>::new(),hints()?,-1i32
    )).await?;
    tokio::time::timeout(std::time::Duration::from_secs(3),async{
        while !marker.exists(){
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),2);
    Ok(())
}

#[tokio::test]
async fn capabilities_union_root_and_scoped_styles_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse(
        "format=%s\nmarkup=0\nactions=0\nicons=0\n         [category=success]\nmarkup=1\nactions=1\nicons=1\n"
    )?;
    let (conn,_runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let capabilities:Vec<String>=desktop.call("GetCapabilities",&()).await?;
    assert_eq!(capabilities,vec![
        "body","body-markup","actions","icon-static",
        "x-canonical-private-synchronous","x-dunst-stack-tag",
    ],"Mako's superstyle includes built-in grouped body plus scoped capabilities");
    Ok(())
}

#[tokio::test]
async fn dbus_set_modes_deduplicates_in_upstream_order()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,_runtime)=open_private_service(Config::default()).await?;
    let control=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    let input=vec![
        "quiet".to_string(),"normal".to_string(),"quiet".to_string(),
        "custom".to_string(),"normal".to_string(),
    ];
    let _:()=control.call("SetModes",&(input,)).await?;
    let listed:Vec<String>=control.call("ListModes",&()).await?;
    assert_eq!(listed,vec!["quiet","normal","custom"]);
    let _:()=control.call("SetModes",&(Vec::<String>::new(),)).await?;
    let empty:Vec<String>=control.call("ListModes",&()).await?;
    assert!(empty.is_empty(),"Mako permits explicitly empty mode sets");
    Ok(())
}
