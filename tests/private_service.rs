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
            Urgency::Low=>0,Urgency::Normal=>1,Urgency::Critical=>2,
        });
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
async fn actions_false_does_not_block_control_or_click_invocation()
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
    let message=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("expected control action signal")?;
    assert_eq!(message.header().member().unwrap().as_str(),"ActionInvoked");
    let (target,key):(u32,String)=message.body().deserialize()?;
    assert_eq!((target,key),(id,"default".to_owned()));
    assert_eq!(runtime.store.lock().unwrap().active().len(),1,
        "Mako control InvokeAction must not dismiss the notification");
    dispatch_click(&conn,&runtime,PointerClick{
        x:30.0,y:20.0,button:272,serial:15,
    }).await?;
    let action=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("expected click action signal")?;
    assert_eq!(action.header().member().unwrap().as_str(),"ActionInvoked");
    let closed=tokio::time::timeout(
        std::time::Duration::from_secs(3),signals.next()
    ).await?.ok_or("expected click close signal")?;
    assert_eq!(closed.header().member().unwrap().as_str(),"NotificationClosed");
    assert!(runtime.store.lock().unwrap().active().is_empty());
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
async fn grouped_on_notify_exec_runs_after_grouping_but_not_on_restore()
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
        "same",0u32,"","Second","",actions,hints,-1i32
    )).await?;
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
async fn capabilities_reflect_mako_root_style_over_dbus()
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let cfg=Config::parse(
        "format=%s\nmarkup=0\nactions=0\nicons=0\n         [category=success]\nmarkup=1\nactions=1\nicons=1\n"
    )?;
    let (conn,_runtime)=open_private_service(cfg).await?;
    let desktop=Proxy::new(&conn,PRIVATE_BUS_NAME,OBJECT_PATH,BUS_NAME).await?;
    let capabilities:Vec<String>=desktop.call("GetCapabilities",&()).await?;
    assert_eq!(capabilities,vec![
        "x-canonical-private-synchronous",
        "x-dunst-stack-tag",
    ],"category-level overrides must not change the advertised root capabilities");
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
