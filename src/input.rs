//! Mako-style mouse bindings for the private Wayland display.
//!
//! A click is translated to an existing notification ID before any mutation.
//! Invalid hit tests, gaps, or unsupported binding values are harmless.
//! No compositor activation token is issued yet: this is not full MVP parity.

use std::io;
use crate::grouping::{group_ordered,GroupSpec};
use crate::hooks::NotifyBinding;
use crate::notification::{CloseReason,Notification};
use crate::runtime::SharedRuntime;
use crate::wayland::PointerClick;
use crate::desktop::style_for_note;
use crate::dbus::{BUS_NAME,OBJECT_PATH};
use zbus::Connection;

fn error(msg: impl std::fmt::Display) -> io::Error {
    io::Error::other(msg.to_string())
}

fn ordered_visible(runtime:&SharedRuntime) -> io::Result<(Vec<Notification>,Vec<String>)> {
    let limit=runtime.config().global.get("max-visible")
        .and_then(|s|s.parse::<usize>().ok()).unwrap_or(5);
    let notes=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?
        .active().iter().take(limit).cloned().collect();
    let modes=runtime.modes().lock().map_err(|_|error("mode lock poisoned"))?
        .clone();
    Ok((notes,modes))
}

/// A click can only target a visible notification's own rectangle, not the
/// transparent gap in the shared layer-shell buffer.
pub fn hit_test(runtime:&SharedRuntime,click:PointerClick)
    -> io::Result<Option<(u32,NotifyBinding)>>
{
    if !click.x.is_finite()||!click.y.is_finite()||click.x<0.0||click.y<0.0 {
        return Ok(None);
    }
    let name=match click.button {
        272=>"on-button-left",
        273=>"on-button-right",
        274=>"on-button-middle",
        _=>return Ok(None),
    };
    let gap=runtime.config().global.get("margin")
        .and_then(|s|s.split(',').next())
        .and_then(|s|s.parse::<u32>().ok()).unwrap_or(10) as f64;
    let (notes,modes)=ordered_visible(runtime)?;
    let mut y=0.0f64;
    for n in &notes {
        let style=style_for_note(runtime,n,&modes);
        let w=style.get("width").and_then(|v|v.parse::<u32>().ok()).unwrap_or(300) as f64;
        let h=style.get("height").and_then(|v|v.parse::<u32>().ok()).unwrap_or(100) as f64;
        if click.y >= y && click.y < y+h {
            if click.x >= w { return Ok(None); }
            let raw=style.get(name).map(String::as_str).unwrap_or("none");
            return NotifyBinding::parse(raw).map(|binding|Some((n.id,binding)))
                .map_err(error);
        }
        y+=h+gap;
    }
    Ok(None)
}

async fn closed(conn:&Connection,runtime:&SharedRuntime,id:u32,history:bool)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    if runtime.close(id,CloseReason::Dismissed,history)? {
        conn.emit_signal(None::<&str>,OBJECT_PATH,BUS_NAME,
            "NotificationClosed",&(id,CloseReason::Dismissed as u32)).await?;
        crate::signals::invalidate(conn,"Notifications").await?;
    }
    Ok(())
}

/// D-Bus signals are emitted from the async task, not the Wayland thread.
pub async fn dispatch_click(
    conn:&Connection,runtime:&SharedRuntime,click:PointerClick,
) -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let Some((id,binding))=hit_test(runtime,click)? else {return Ok(())};
    match binding {
        NotifyBinding::None=>{}
        NotifyBinding::Exec(cmd)=>NotifyBinding::Exec(cmd).run_exec(id)?,
        NotifyBinding::Dismiss {save_history}=>closed(conn,runtime,id,save_history).await?,
        NotifyBinding::DismissAll=>{
            let ids:Vec<_>=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?
                .active().iter().map(|n|n.id).collect();
            for id in ids {closed(conn,runtime,id,true).await?;}
        }
        NotifyBinding::DismissGroup=>{
            let raw=runtime.config().global.get("group-by")
                .map(String::as_str).unwrap_or("none");
            let spec=GroupSpec::parse(raw).map_err(error)?;
            let ids={
                let store=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?;
                group_ordered(store.active(),&spec).into_iter()
                    .find(|g|g.ids.contains(&id)).map(|g|g.ids).unwrap_or_default()
            };
            for id in ids {closed(conn,runtime,id,true).await?;}
        }
        NotifyBinding::InvokeDefaultAction=>invoke(conn,runtime,id,"default").await?,
        NotifyBinding::InvokeAction(key)=>invoke(conn,runtime,id,&key).await?,
    }
    Ok(())
}

async fn invoke(conn:&Connection,runtime:&SharedRuntime,id:u32,key:&str)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let has_action=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?
        .active().iter().find(|n|n.id==id)
        .is_some_and(|n|n.actions.iter().any(|a|a.key==key));
    if has_action {
        conn.emit_signal(None::<&str>,OBJECT_PATH,BUS_NAME,
            "ActionInvoked",&(id,key)).await?;
    }
    // Match Mako: even the default action with no matching key dismisses.
    closed(conn,runtime,id,true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::notification::Request;
    fn click(x:f64,y:f64,button:u32)->PointerClick{
        PointerClick{x,y,button,serial:3}
    }
    #[test]
    fn dispatch_targets_only_rectangles_and_correct_mouse_binding() {
        let cfg=Config::parse(
            "width=100\nheight=40\nmargin=5\non-button-left=invoke-default-action\n             on-button-right=dismiss\n[category=warning]\non-button-left=none\n"
        ).unwrap();
        let runtime=SharedRuntime::new(cfg);
        let old=runtime.store.lock().unwrap().push(Request::default());
        let newest=runtime.store.lock().unwrap().push(Request {
            category:"warning".into(),..Request::default()
        });
        assert_eq!(hit_test(&runtime,click(90.0,10.0,272)).unwrap(),
            Some((newest,NotifyBinding::None)));
        assert_eq!(hit_test(&runtime,click(90.0,10.0,273)).unwrap(),
            Some((newest,NotifyBinding::Dismiss{save_history:true})));
        assert_eq!(hit_test(&runtime,click(90.0,42.0,272)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(90.0,50.0,272)).unwrap(),
            Some((old,NotifyBinding::InvokeDefaultAction)));
        assert_eq!(hit_test(&runtime,click(100.0,10.0,272)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(0.0,10.0,999)).unwrap(),None);
    }

    #[test]
    fn malformed_pointer_coordinates_are_not_clickable() {
        let runtime=SharedRuntime::new(Config::default());
        runtime.store.lock().unwrap().push(Request::default());
        for x in [f64::NAN,f64::INFINITY,f64::NEG_INFINITY,-1.0] {
            assert!(hit_test(&runtime,click(x,3.0,272)).unwrap().is_none());
        }
    }
}
