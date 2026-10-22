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
use crate::desktop::{style_for_note_index,visible_members_indexed,stack_alignment};
use crate::wayland::PopupMargins;
use crate::render::mako_stack_geometry;
use crate::dbus::{BUS_NAME,OBJECT_PATH};
use zbus::Connection;

fn error(msg: impl std::fmt::Display) -> io::Error {
    io::Error::other(msg.to_string())
}

fn ordered_visible(runtime:&SharedRuntime)
    -> io::Result<(Vec<crate::desktop::VisibleMember>,Vec<String>)>
{
    let notes=visible_members_indexed(runtime)?;
    let modes=runtime.modes().lock().map_err(|_|error("mode lock poisoned"))?
        .clone();
    Ok((notes,modes))
}

/// A click can only target a visible notification's own rectangle, not the
/// transparent gap in the shared layer-shell buffer.
pub fn hit_test(runtime:&SharedRuntime,click:PointerClick)
    -> io::Result<Option<(u32,NotifyBinding)>>
{
    let config=runtime.config();
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
    let dimensions:Vec<_>=notes.iter().map(|entry|{
        let n=&entry.notification;
        let style=style_for_note_index(runtime,n,&modes,entry.group_index);
        margins.push(style.get("margin")
            .and_then(|s|PopupMargins::parse_mako(s))
            .unwrap_or(PopupMargins::uniform(10)));
        let w=style.get("width").and_then(|v|v.parse::<u32>().ok()).unwrap_or(300) as f64;
        // Hit-testing must use the same Pango shrink-wrapped height as
        // the renderer, not the configured maximum.
        let h=crate::render::measure_notification_height_group(
            n,&style,if entry.group_count>=2{entry.group_count}else{0}
        ).map_err(error)? as f64;
        Ok::<(f64,f64),io::Error>((w,h))
    }).collect::<io::Result<Vec<_>>>()?;
    let align=stack_alignment(config.global.get("anchor")
        .map(String::as_str).unwrap_or("top-right"));
    if dimensions.is_empty(){return Ok(None);}
    let sizes:Vec<_>=dimensions.iter().map(|(w,h)|(*w as u32,*h as u32)).collect();
    let (_,_,positions)=mako_stack_geometry(&sizes,&margins,align).map_err(error)?;
    for ((entry,(w,h)),(x,y)) in notes.iter()
        .zip(dimensions.iter().copied()).zip(positions) {
        let (x,y)=(f64::from(x),f64::from(y));
        let n=&entry.notification;
        let style=style_for_note_index(runtime,n,&modes,entry.group_index);
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
    dispatch_click_with_activation(conn,runtime,click,None).await
}

/// Activation tokens originate only from compositor-issued click serials,
/// never from notification content or control API calls.
pub async fn dispatch_click_with_activation(
    conn:&Connection,runtime:&SharedRuntime,click:PointerClick,
    activation_token:Option<&str>,
) -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let config=runtime.config();
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
            let raw=config.global.get("group-by")
                .map(String::as_str).unwrap_or("none");
            let spec=GroupSpec::parse(raw).map_err(error)?;
            let ids={
                let store=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?;
                group_ordered(store.active(),&spec).into_iter()
                    .find(|g|g.ids.contains(&id)).map(|g|g.ids).unwrap_or_default()
            };
            for id in ids {closed(conn,runtime,id,true).await?;}
        }
        NotifyBinding::InvokeDefaultAction=>
            invoke(conn,runtime,id,"default",activation_token).await?,
        NotifyBinding::InvokeAction(key)=>
            invoke(conn,runtime,id,&key,activation_token).await?,
    }
    Ok(())
}

/// A compositor token is useful only for action-invoking bindings. Avoid an
/// expensive Wayland roundtrip on dismiss/none and external-command clicks.
pub fn needs_activation_token(binding:&NotifyBinding)->bool{
    matches!(binding,NotifyBinding::InvokeDefaultAction|NotifyBinding::InvokeAction(_))
}

/// The Mako actions=0 option suppresses ActionInvoked, while the matching
/// click binding may still dismiss the notification.
pub fn can_invoke_action(runtime:&SharedRuntime,id:u32,key:&str)->io::Result<bool>{
    let config=runtime.config();
    let (note,group_index)={
        let store=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?;
        let note=store.active().iter().find(|n|n.id==id).cloned();
        let grouping=config.global.get("group-by")
            .map(String::as_str).unwrap_or("none");
        let spec=GroupSpec::parse(grouping).map_err(error)?;
        let index=group_ordered(store.active(),&spec).iter()
            .find(|group|group.ids.contains(&id))
            .and_then(|group|group.index_of(id)).unwrap_or(-1);
        (note,index)
    };
    let Some(note)=note else {return Ok(false)};
    let modes=runtime.modes().lock().map_err(|_|error("mode lock poisoned"))?
        .clone();
    let style=style_for_note_index(runtime,&note,&modes,group_index);
    Ok(style.get("actions").is_none_or(|v|
        v=="1"||v.eq_ignore_ascii_case("true"))
        && note.actions.iter().any(|a|a.key==key))
}

async fn invoke(conn:&Connection,runtime:&SharedRuntime,id:u32,key:&str,
    activation_token:Option<&str>)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    if can_invoke_action(runtime,id,key)? {
        if let Some(token)=activation_token.filter(|token|!token.is_empty()) {
            conn.emit_signal(None::<&str>,OBJECT_PATH,BUS_NAME,
                "ActivationToken",&(id,token)).await?;
        }
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
        let top={
            let store=runtime.store.lock().unwrap();
            let note=store.active().iter().find(|n|n.id==newest).unwrap();
            crate::render::measure_notification_height(
                note,&crate::desktop::style_for_note(
                    &runtime,note,&["default".into()]
                )
            ).unwrap()
        };
        assert_eq!(hit_test(&runtime,click(90.0,f64::from(top)+7.0,272)).unwrap(),None,
            "adjacent notification margins are transparent");
        assert_eq!(hit_test(&runtime,click(90.0,f64::from(top)+15.0,272)).unwrap(),
            Some((old,NotifyBinding::InvokeDefaultAction)));
        assert_eq!(hit_test(&runtime,click(105.0,10.0,272)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(2.0,10.0,272)).unwrap(),None);
        assert_eq!(hit_test(&runtime,click(0.0,10.0,999)).unwrap(),None);
    }

    #[test]
    fn actions_disabled_by_category_do_not_emit_invocation() {
        let runtime=SharedRuntime::new(Config::parse(
            "[category=warning]\nactions=0\n"
        ).unwrap());
        let disabled=runtime.store.lock().unwrap().push(Request{
            category:"warning".into(),
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],..Request::default()
        });
        let allowed=runtime.store.lock().unwrap().push(Request{
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],..Request::default()
        });
        assert!(!can_invoke_action(&runtime,disabled,"default").unwrap());
        assert!(can_invoke_action(&runtime,allowed,"default").unwrap());
        assert!(!can_invoke_action(&runtime,allowed,"unknown").unwrap());
    }

    #[test]
    fn exposed_secondary_group_member_uses_its_own_input_binding() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\nmax-visible=4\nwidth=120\nheight=40\n             [grouped]\ninvisible=0\n             [group-index=1]\non-button-left=none\n"
        ).unwrap());
        let older=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        let newer=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        let members=visible_members_indexed(&runtime).unwrap();
        assert_eq!(members.len(),2);
        let first=&members[0];
        let first_style=style_for_note_index(&runtime,&first.notification,
            &["default".into()],first.group_index);
        let top_height=crate::render::measure_notification_height_group(
            &first.notification,&first_style,2).unwrap();
        assert_eq!(hit_test(&runtime,click(20.0,15.0,272)).unwrap(),
            Some((newer,NotifyBinding::InvokeDefaultAction)));
        assert_eq!(hit_test(&runtime,click(20.0,f64::from(top_height)+22.0,272)).unwrap(),
            Some((older,NotifyBinding::None)));
    }

    #[test]
    fn grouped_actions_disabled_criterion_suppresses_signal() {
        let config=Config::parse(
            "group-by=app-name\n[grouped]\nactions=0\n"
        ).unwrap();
        let runtime=SharedRuntime::new(config);
        let request=||Request{
            app_name:"grouped-app".into(),
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],
            ..Request::default()
        };
        let first=runtime.store.lock().unwrap().push(request());
        assert!(can_invoke_action(&runtime,first,"default").unwrap());
        let second=runtime.store.lock().unwrap().push(request());
        assert!(!can_invoke_action(&runtime,first,"default").unwrap());
        assert!(!can_invoke_action(&runtime,second,"default").unwrap());
    }

    #[test]
    fn activation_roundtrip_is_reserved_for_action_bindings() {
        assert!(needs_activation_token(&NotifyBinding::InvokeDefaultAction));
        assert!(needs_activation_token(&NotifyBinding::InvokeAction("open".into())));
        assert!(!needs_activation_token(&NotifyBinding::None));
        assert!(!needs_activation_token(&NotifyBinding::Dismiss{save_history:true}));
        assert!(!needs_activation_token(&NotifyBinding::DismissAll));
        assert!(!needs_activation_token(&NotifyBinding::Exec("true".into())));
    }

    #[test]
    fn action_permission_boolean_is_case_insensitive_like_mako_config() {
        let runtime=SharedRuntime::new(Config::parse("actions=TRUE\n").unwrap());
        let id=runtime.store.lock().unwrap().push(Request{
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],..Request::default()
        });
        assert!(can_invoke_action(&runtime,id,"default").unwrap());
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
            assert_eq!(hit_test(&runtime,click(15.0,15.0,button)).unwrap(),
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
        let top={
            let store=runtime.store.lock().unwrap();
            let n=store.active().iter().find(|n|n.id==newer).unwrap();
            let style=crate::desktop::style_for_note(&runtime,n,&["default".into()]);
            crate::render::measure_notification_height(n,&style).unwrap()
        };
        assert_eq!(hit_test(&runtime,click(30.0,7.0,273)).unwrap().unwrap().0,newer);
        assert!(hit_test(&runtime,click(30.0,f64::from(top)+6.0,273)).unwrap().is_none());
        assert_eq!(hit_test(&runtime,click(30.0,f64::from(top)+8.0,273)).unwrap().unwrap().0,older);
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
        let top={
            let store=runtime.store.lock().unwrap();
            let n=store.active().iter().find(|n|n.id==narrow).unwrap();
            let style=crate::desktop::style_for_note(&runtime,n,&["default".into()]);
            crate::render::measure_notification_height(n,&style).unwrap()
        };
        assert_eq!(hit_test(&runtime,click(10.0,f64::from(top)+10.0,273)).unwrap(),
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
