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
async fn standard_notification_name_is_claimed_only_inside_test_session()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // CI creates an isolated dbus-run-session; this never competes with the
    // real workstation Mako notification daemon.
    let (conn,runtime)=posthorn::service::open_standard_service(Config::default()).await?;
    let desktop=Proxy::new(&conn,BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let id:u32=desktop.call("Notify",&(
        "standard-bus-test",0u32,"","Title","Body",
        Vec::<String>::new(),HashMap::<String,OwnedValue>::new(),-1i32
    )).await?;
    assert_eq!(runtime.store.lock().unwrap().active().len(),1);
    assert_eq!(runtime.store.lock().unwrap().active()[0].id,id);
    Ok(())
}
