//! Explicitly opt-in **headless** D-Bus daemon test harness.
//!
//! Normal invocations take only org.posthorn.Development, never
//! org.freedesktop.Notifications. The latter is exercised solely by the
//! explicitly called CI helper under an isolated dbus-run-session.
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
    // Set up the *whole* object before owning any well-known name. zbus
    // Builder::name requests ownership before this post-build property
    // override is installed; callers can observe a lossy Notifications
    // property in that interval. Default name requests may also queue.
    let conn = connection::Builder::session()?
        .serve_at(OBJECT_PATH, FreedesktopNotifications::with_runtime(runtime.clone()))?
        .serve_at(CONTROL_PATH, MakoControl::with_runtime(runtime.clone()))?
        .build()
        .await?;
    let server=conn.object_server();
    server.remove::<zbus::fdo::Properties,_>(CONTROL_PATH).await?;
    let registered=server.at(CONTROL_PATH,
        crate::control::OrderedControlProperties::with_runtime(runtime.clone())).await?;
    if !registered {
        return Err(zbus::Error::Failure(
            "unable to register ordered Mako control properties".into()));
    }
    // Never replace or queue behind an existing notification daemon. If
    // another service owns the name, return NameTaken immediately. This
    // is especially important for eventual opt-in reversible cutover.
    conn.request_name_with_flags(
        name,zbus::fdo::RequestNameFlags::DoNotQueue.into()
    ).await?;
    Ok((conn, runtime))
}

/// A private service with reload enabled for its explicit config path.
pub async fn open_private_service_from_path(path:&Path)
    -> Result<(Connection,SharedRuntime),Box<dyn std::error::Error+Send+Sync>>
{
    open_private_service_from_path_with_cli_overrides(path,&[]).await
}

/// Exercise original Mako CLI style precedence on a disposable private bus.
/// This is not a production startup path and cannot seize the real name.
pub async fn open_private_service_from_path_with_cli_overrides(
    path:&Path,overrides:&[(String,String)]
)->Result<(Connection,SharedRuntime),Box<dyn std::error::Error+Send+Sync>>{
    let runtime=SharedRuntime::from_path_with_cli_overrides(path,overrides)?;
    Ok(open_named_runtime(runtime,PRIVATE_BUS_NAME).await?)
}

/// Construct the private test daemon. It never uses Mako's real bus name.
pub async fn open_private_service(config: Config) -> zbus::Result<(Connection, SharedRuntime)> {
    open_named_service(config, PRIVATE_BUS_NAME).await
}

/// The private Wayland desktop learns monitor scale BEFORE requesting its
/// development D-Bus name, so even the earliest Notify uses the right theme.
pub(crate) async fn open_private_runtime(
    runtime:SharedRuntime
)->zbus::Result<(Connection,SharedRuntime)>{
    open_named_runtime(runtime,PRIVATE_BUS_NAME).await
}

/// Standard-name registration exclusively for the separately gated
/// reversible trial. The existing registration refuses to queue or
/// displace an owned name, and all D-Bus objects are ready beforehand.
pub(crate) async fn open_standard_trial_runtime(runtime:SharedRuntime)
    ->zbus::Result<(Connection,SharedRuntime)>{
    // The second gate prevents bypass via a direct library helper call.
    // A caller cannot acquire the public name merely by constructing a
    // valid SharedRuntime; only a separately armed bus-pinned trial may.
    let authorized=std::env::var("POSTHORN_PRODUCTION_TRIAL_AUTHORIZED")
        .ok().as_deref()==Some("1");
    let pinned=std::env::var("POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS")
        .unwrap_or_default();
    let active=std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    let private_qa=std::env::var("POSTHORN_HYPRLAND_QA")
        .ok().as_deref()==Some("1");
    if !authorized || pinned.is_empty() || pinned!=active || private_qa{
        return Err(zbus::Error::Failure(
            "public-name trial requires separate authorization and matching bus pin".into(),
        ));
    }
    open_named_runtime(runtime,BUS_NAME).await
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
    let expired = runtime.expire_with_changes(std::time::Instant::now())?;
    for (id,changed) in &expired {
        conn.emit_signal(
            None::<&str>, OBJECT_PATH, BUS_NAME, "NotificationClosed",
            &(*id, CloseReason::Expired as u32),
        ).await?;
        if *changed{
            crate::signals::invalidate(conn, "Notifications").await?;
        }
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
    run_private_service_from_path_with_cli_overrides(path,&[]).await
}

/// Only the opt-in private development name is available to the CLI-aware
/// runtime. Invalid flags/config reject before requesting any D-Bus name.
pub async fn run_private_service_from_path_with_cli_overrides(
    path:&Path,overrides:&[(String,String)]
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let (conn,runtime)=open_private_service_from_path_with_cli_overrides(
        path,overrides).await?;
    run_private_service_instance(conn,runtime).await
}

pub async fn run_private_service_from_defaults_with_cli_overrides(
    overrides:&[(String,String)],
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let runtime=SharedRuntime::from_mako_defaults_with_cli_overrides(overrides)?;
    let (conn,runtime)=open_private_runtime(runtime).await?;
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
