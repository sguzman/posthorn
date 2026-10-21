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
    assert!(runtime.next_deadline()?.is_some(),
        "restored notification must be rescheduled");
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
    let failed:Result<(),_>=ctl.call("Reload",&()).await;
    assert!(failed.is_err(),"invalid reload must be rejected");
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
