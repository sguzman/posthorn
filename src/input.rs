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
use crate::desktop::{style_for_note,stack_alignment,vertical_spacing};
use crate::wayland::PopupMargins;
use crate::render::StackAlignment;
use crate::dbus::{BUS_NAME,OBJECT_PATH};
use zbus::Connection;

fn error(msg: impl std::fmt::Display) -> io::Error {
    io::Error::other(msg.to_string())
}

fn ordered_visible(runtime:&SharedRuntime) -> io::Result<(Vec<Notification>,Vec<String>)> {
    let notes=crate::desktop::visible_notifications(runtime)?;
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
        0=>"on-touch",
        272=>"on-button-left",
        273=>"on-button-right",
        274=>"on-button-middle",
        _=>return Ok(None),
    };
    let (notes,modes)=ordered_visible(runtime)?;
    // Layer-shell's surface width is the largest notification width.
    // Narrow notifications may be left, centered, or right aligned.
    let mut margins=Vec::with_capacity(notes.len());
    let dimensions:Vec<_>=notes.iter().map(|n|{
        let style=style_for_note(runtime,n,&modes);
        margins.push(style.get("margin")
            .and_then(|s|PopupMargins::parse_mako(s))
            .unwrap_or(PopupMargins::uniform(10)));
        let w=style.get("width").and_then(|v|v.parse::<u32>().ok()).unwrap_or(300) as f64;
        // Hit-testing must use the same Pango shrink-wrapped height as
        // the renderer, not the configured maximum.
        let h=crate::render::measure_notification_height(n,&style)
            .map_err(error)? as f64;
        (w,h)
    }).collect();
    let gaps=vertical_spacing(&margins);
    let max_width=dimensions.iter().map(|(w,_)|*w).fold(0.0f64,f64::max);
    let align=stack_alignment(runtime.config().global.get("anchor")
        .map(String::as_str).unwrap_or("top-right"));
    let mut y=0.0f64;
    for (index,(n,(w,h))) in notes.iter().zip(dimensions.iter().copied()).enumerate() {
        let style=style_for_note(runtime,n,&modes);
        let x=match align {
            StackAlignment::Left=>0.0,
            StackAlignment::Center=>(max_width-w)/2.0,
            StackAlignment::Right=>max_width-w,
        };
        if click.y >= y && click.y < y+h {
            if click.x<x || click.x>=x+w {return Ok(None);}
            let fallback=match click.button {
                272=>"invoke-default-action",
                273|0=>"dismiss",
                _=>"none",
            };
            let raw=style.get(name).map(String::as_str).unwrap_or(fallback);
            return NotifyBinding::parse(raw).map(|binding|Some((n.id,binding)))
                .map_err(error);
        }
        y+=h+f64::from(gaps.get(index).copied().unwrap_or(0));
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
        assert_eq!(hit_test(&runtime,click(90.0,10.0,0)).unwrap(),
            Some((newest,NotifyBinding::Dismiss{save_history:true})));
        assert_eq!(hit_test(&runtime,click(90.0,42.0,272)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(90.0,50.0,272)).unwrap(),
            Some((old,NotifyBinding::InvokeDefaultAction)));
        assert_eq!(hit_test(&runtime,click(100.0,10.0,272)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(0.0,10.0,999)).unwrap(),None);
    }

    #[test]
    fn upstream_mako_default_buttons_work_without_user_config() {
        let runtime=SharedRuntime::new(Config::default());
        let id=runtime.store.lock().unwrap().push(Request::default());
        for (button,binding) in [
            (272,NotifyBinding::InvokeDefaultAction),
            (273,NotifyBinding::Dismiss{save_history:true}),
            (274,NotifyBinding::None),
            (0,NotifyBinding::Dismiss{save_history:true}),
        ] {
            assert_eq!(hit_test(&runtime,click(5.0,5.0,button)).unwrap(),
                Some((id,binding)));
        }
    }

    #[test]
    fn mouse_targets_follow_asymmetric_vertical_spacers() {
        let runtime=SharedRuntime::new(Config::parse(
            "width=120\nheight=30\nmargin=2,1,12,1\n             [category=warning]\nmargin=5,1,1,1\n"
        ).unwrap());
        let older=runtime.store.lock().unwrap().push(Request::default());
        let newer=runtime.store.lock().unwrap().push(Request{
            category:"warning".into(),..Request::default()
        });
        // Newer bottom margin=1, older top margin=2 -> 2px gap.
        assert_eq!(hit_test(&runtime,click(30.0,15.0,273)).unwrap().unwrap().0,newer);
        assert!(hit_test(&runtime,click(30.0,31.0,273)).unwrap().is_none());
        assert_eq!(hit_test(&runtime,click(30.0,33.0,273)).unwrap().unwrap().0,older);
    }

    #[test]
    fn hit_testing_accounts_for_right_aligned_variable_width_popups() {
        let runtime=SharedRuntime::new(Config::parse(
            "width=240\nheight=40\nmargin=4\nanchor=top-right\n             [category=warning]\nwidth=90\n"
        ).unwrap());
        let wide=runtime.store.lock().unwrap().push(Request::default());
        let narrow=runtime.store.lock().unwrap().push(Request{
            category:"warning".into(),..Request::default()
        });
        assert_eq!(hit_test(&runtime,click(10.0,10.0,273)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(200.0,10.0,273)).unwrap(),
            Some((narrow,NotifyBinding::Dismiss{save_history:true})));
        assert_eq!(hit_test(&runtime,click(10.0,50.0,273)).unwrap(),
            Some((wide,NotifyBinding::Dismiss{save_history:true})));
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
