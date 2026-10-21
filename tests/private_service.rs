#![cfg(feature = "dbus-integration")]
//! End-to-end D-Bus lifecycle through a safely isolated private name.

use std::collections::HashMap;
use std::time::Duration;
use posthorn::config::Config;
use posthorn::control::{CONTROL_INTERFACE, CONTROL_PATH};
use posthorn::dbus::{BUS_NAME, OBJECT_PATH};
use posthorn::service::{open_private_service, tick_expirations, PRIVATE_BUS_NAME};
use zbus::{Proxy, zvariant::OwnedValue};

#[tokio::test]
async fn private_service_expires_and_archives_a_notification() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::parse(
        "default-timeout=35\nignore-timeout=1\nmax-history=3\n"
    )?;
    let (conn, runtime) = open_private_service(config).await?;
    let desktop = Proxy::new(&conn, PRIVATE_BUS_NAME, OBJECT_PATH, BUS_NAME).await?;
    let control = Proxy::new(&conn, PRIVATE_BUS_NAME, CONTROL_PATH, CONTROL_INTERFACE).await?;
    let id: u32 = desktop.call("Notify", &(
        "test-sender", 0u32, "", "Done", "body", Vec::<String>::new(),
        HashMap::<String, OwnedValue>::new(), 100000i32
    )).await?;
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
    execute(Command::Dismiss {
        id, all: false, group: false, history: false
    }).await?;
    let dismissed: Vec<HashMap<String, OwnedValue>> =
        control.call("ListNotifications", &()).await?;
    assert!(dismissed.is_empty());
    assert!(runtime.next_deadline()?.is_none(),
        "dismissed notification must not retain an expiration timer");
    execute(Command::Restore).await?;
    assert!(runtime.next_deadline()?.is_some(),
        "restored notification must be rescheduled");
    Ok(())
}
