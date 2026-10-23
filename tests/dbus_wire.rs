#![cfg(feature = "dbus-integration")]
//! Real D-Bus wire-level tests, run under dbus-run-session in CI.
//! A private test name ensures these tests NEVER claim the live desktop
//! org.freedesktop.Notifications name or interfere with the installed Mako.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use posthorn::config::Config;
use posthorn::control::{MakoControl, CONTROL_INTERFACE, CONTROL_PATH};
use posthorn::dbus::{FreedesktopNotifications, BUS_NAME, OBJECT_PATH};
use posthorn::notification::NotificationStore;
use zbus::{connection, zvariant::OwnedValue, Proxy};

const TEST_NAME: &str = "org.posthorn.WireTest";

#[tokio::test]
async fn accepts_notify_then_exposes_control_list_and_history() -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(Mutex::new(NotificationStore::new(5)));
    let service = FreedesktopNotifications::new(Arc::clone(&state));
    let control = MakoControl::new(Arc::clone(&state));
    let conn = connection::Builder::session()?
        .name(TEST_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .serve_at(CONTROL_PATH, control)?
        .build().await?;

    let desktop = Proxy::new(&conn, TEST_NAME, OBJECT_PATH, BUS_NAME).await?;
    let modes = Proxy::new(&conn, TEST_NAME, CONTROL_PATH, CONTROL_INTERFACE).await?;
    let hints = HashMap::<String, OwnedValue>::new();
    let action_pairs = Vec::<String>::new();

    let id: u32 = desktop.call(
        "Notify", &("fixture-app", 0u32, "", "Example", "Synthetic test",
            action_pairs, hints, 7000i32)
    ).await?;
    assert!(id > 0);

    let list: Vec<HashMap<String, OwnedValue>> =
        modes.call("ListNotifications", &()).await?;
    assert_eq!(list.len(), 1);
    assert_eq!(u32::try_from(list[0].get("id").unwrap())?, id);

    let initial: Vec<String> = modes.call("ListModes", &()).await?;
    assert_eq!(initial, vec!["default"]);
    let _: () = modes.call("SetModes", &(vec!["quiet".to_string()])).await?;
    let after: Vec<String> = modes.call("ListModes", &()).await?;
    assert_eq!(after, vec!["quiet"]);

    let _: () = desktop.call("CloseNotification", &(id,)).await?;
    let current: Vec<HashMap<String, OwnedValue>> =
        modes.call("ListNotifications", &()).await?;
    let history: Vec<HashMap<String, OwnedValue>> =
        modes.call("ListHistory", &()).await?;
    assert!(current.is_empty());
    assert_eq!(history.len(), 1);
    assert_eq!(u32::try_from(history[0].get("id").unwrap())?, id);

    Ok(())
}

#[tokio::test]
async fn private_service_does_not_queue_behind_an_unrelated_owner()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    use posthorn::service::{open_private_service,PRIVATE_BUS_NAME};
    // Simulate another application holding the private development name.
    // The service must neither replace that owner nor join its queue.
    let owner=connection::Builder::session()?
        .name(PRIVATE_BUS_NAME)?
        .build().await?;
    let owner_name=owner.unique_name().ok_or("owner has no unique name")?.to_string();
    let dbus=zbus::fdo::DBusProxy::new(&owner).await?;
    let attempt=tokio::time::timeout(std::time::Duration::from_secs(2),
        open_private_service(Config::default())).await
        .expect("Posthorn blocked on a name already held by another process");
    assert!(matches!(attempt,Err(zbus::Error::NameTaken)));
    assert_eq!(dbus.get_name_owner(PRIVATE_BUS_NAME.try_into()?).await?.as_str(),
        owner_name,"Posthorn must not replace an unrelated D-Bus owner");
    Ok(())
}

#[tokio::test]
async fn standard_name_collision_fails_without_queueing_or_replacing_owner()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // Only under CI's dbus-run-session; never claim the user's real bus name.
    let (first,_runtime)=posthorn::service::open_standard_service(Config::default()).await?;
    let first_owner=first.unique_name().ok_or("missing first unique bus name")?.to_string();
    let dbus=zbus::fdo::DBusProxy::new(&first).await?;
    assert_eq!(dbus.get_name_owner(BUS_NAME.try_into()?).await?.as_str(),first_owner);

    let second=tokio::time::timeout(
        std::time::Duration::from_secs(2),
        posthorn::service::open_standard_service(Config::default())
    ).await.expect("second service queued instead of immediately rejecting the name");
    assert!(matches!(second,Err(zbus::Error::NameTaken)),
        "a second Posthorn instance must fail without replacing Mako");
    assert_eq!(dbus.get_name_owner(BUS_NAME.try_into()?).await?.as_str(),first_owner,
        "an unsuccessful startup must not transfer ownership");
    Ok(())
}

#[tokio::test]
async fn standard_notification_name_is_claimed_only_inside_test_session()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // CI creates an isolated dbus-run-session; this never competes with the
    // real workstation Mako notification daemon.
    let (conn,runtime)=posthorn::service::open_standard_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let id:u32=desktop.call("Notify",&(
        "standard-bus-test",0u32,"","Title","Body",
        vec![
            "same".to_string(),"Older-label-1".to_string(),
            "same".to_string(),"Newer-label-2".to_string(),
        ],
        HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    assert_eq!(runtime.store.lock().unwrap().active()[0].id,id);
    // The real public name is only claimed under dbus-run-session. Verify
    // that the ordered Properties adapter is installed for this eventual
    // production-facing name as well, never on the user's session bus.
    let props=Proxy::new(&conn,BUS_NAME,CONTROL_PATH,
        "org.freedesktop.DBus.Properties").await?;
    for (method,args) in [
        ("Get",vec![CONTROL_INTERFACE,"Notifications"]),
        ("GetAll",vec![CONTROL_INTERFACE]),
    ] {
        let reply=if method=="Get" {
            props.call_method(method,&(args[0],args[1])).await?
        } else {
            props.call_method(method,&(args[0],)).await?
        };
        let bytes=reply.body().data().bytes().to_vec();
        let older=bytes.windows(b"Older-label-1\0".len())
            .position(|w|w==b"Older-label-1\0").unwrap();
        let newer=bytes.windows(b"Newer-label-2\0".len())
            .position(|w|w==b"Newer-label-2\0").unwrap();
        assert!(newer<older,
            "{method} on the standard test bus must retain duplicate actions");
    }
    Ok(())
}
