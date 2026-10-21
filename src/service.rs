//! Explicitly opt-in **headless** D-Bus daemon test harness.
//!
//! This deliberately takes org.posthorn.Development, never
//! org.freedesktop.Notifications, so it cannot replace Mako by accident.
//! Display integration and final compositor behavior remain future MVP work.

use std::time::Duration;

use zbus::{connection, Connection};

use crate::config::Config;
use crate::control::{MakoControl, CONTROL_PATH};
use crate::dbus::{FreedesktopNotifications, BUS_NAME, OBJECT_PATH};
use crate::notification::CloseReason;
use crate::runtime::SharedRuntime;

pub const PRIVATE_BUS_NAME: &str = "org.posthorn.Development";

/// Construct a functional test service on its own D-Bus destination.
pub async fn open_private_service(config: Config) -> zbus::Result<(Connection, SharedRuntime)> {
    let runtime = SharedRuntime::new(config);
    let conn = connection::Builder::session()?
        .name(PRIVATE_BUS_NAME)?
        .serve_at(OBJECT_PATH, FreedesktopNotifications::with_runtime(runtime.clone()))?
        .serve_at(CONTROL_PATH, MakoControl::with_runtime(runtime.clone()))?
        .build()
        .await?;
    Ok((conn, runtime))
}

/// Emit expiry signals only for notifications that actually transitioned from
/// active to history. Test buses exercise real D-Bus serialization.
pub async fn tick_expirations(conn: &Connection, runtime: &SharedRuntime)
    -> Result<usize, Box<dyn std::error::Error + Send + Sync>>
{
    let expired = runtime.expire(std::time::Instant::now())?;
    for id in &expired {
        conn.emit_signal(
            None::<&str>, OBJECT_PATH, BUS_NAME, "NotificationClosed",
            &(*id, CloseReason::Expired as u32),
        ).await?;
    }
    Ok(expired.len())
}

pub async fn run_private_service(config: Config)
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let (conn, runtime) = open_private_service(config).await?;
    eprintln!(
        "Posthorn headless development service on {PRIVATE_BUS_NAME};          no Wayland popups; Mako remains the desktop notification daemon"
    );
    let mut timer = tokio::time::interval(Duration::from_millis(25));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        tick_expirations(&conn, &runtime).await?;
    }
}
