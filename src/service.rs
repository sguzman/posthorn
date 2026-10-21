//! Explicitly opt-in **headless** D-Bus daemon test harness.
//!
//! This deliberately takes org.posthorn.Development, never
//! org.freedesktop.Notifications, so it cannot replace Mako by accident.
//! Display integration and final compositor behavior remain future MVP work.

use std::path::Path;
use std::time::Duration;

use zbus::{connection, Connection};

use crate::config::Config;
use crate::control::{MakoControl, CONTROL_PATH};
use crate::dbus::{FreedesktopNotifications, BUS_NAME, OBJECT_PATH};
use crate::notification::CloseReason;
use crate::runtime::SharedRuntime;

pub const PRIVATE_BUS_NAME: &str = "org.posthorn.Development";

/// Construct a functional service using an explicitly selected name.
/// Production registration is NEVER attempted by the default Posthorn CLI.
async fn open_named_service(config: Config, name: &str)
    -> zbus::Result<(Connection, SharedRuntime)>
{
    open_named_runtime(SharedRuntime::new(config),name).await
}

async fn open_named_runtime(runtime:SharedRuntime,name:&str)
    -> zbus::Result<(Connection,SharedRuntime)>
{
    let conn = connection::Builder::session()?
        .name(name)?
        .serve_at(OBJECT_PATH, FreedesktopNotifications::with_runtime(runtime.clone()))?
        .serve_at(CONTROL_PATH, MakoControl::with_runtime(runtime.clone()))?
        .build()
        .await?;
    Ok((conn, runtime))
}

/// A private service with reload enabled for its explicit config path.
pub async fn open_private_service_from_path(path:&Path)
    -> Result<(Connection,SharedRuntime),Box<dyn std::error::Error+Send+Sync>>
{
    let runtime=SharedRuntime::from_path(path)?;
    Ok(open_named_runtime(runtime,PRIVATE_BUS_NAME).await?)
}

/// Construct the private test daemon. It never uses Mako's real bus name.
pub async fn open_private_service(config: Config) -> zbus::Result<(Connection, SharedRuntime)> {
    open_named_service(config, PRIVATE_BUS_NAME).await
}

/// Prepare the eventual desktop notification server. This method is *only*
/// exercised under dbus-run-session in CI; no startup command invokes it yet.
/// A full replacement is forbidden until the MVP parity gate is complete.
pub async fn open_standard_service(config: Config) -> zbus::Result<(Connection, SharedRuntime)> {
    open_named_service(config, BUS_NAME).await
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
    if !expired.is_empty() {
        crate::signals::invalidate(conn, "Notifications").await?;
    }
    Ok(expired.len())
}

pub async fn run_private_service(config: Config)
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let (conn, runtime) = open_private_service(config).await?;
    run_private_service_instance(conn,runtime).await
}

pub async fn run_private_service_from_path(path:&Path)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=open_private_service_from_path(path).await?;
    run_private_service_instance(conn,runtime).await
}

async fn run_private_service_instance(conn:Connection,runtime:SharedRuntime)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
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
