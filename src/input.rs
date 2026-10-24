//! Mako-style mouse bindings for the private Wayland display.
//!
//! A click is translated to an existing notification ID before any mutation.
//! Invalid hit tests, gaps, or unsupported binding values are harmless.
//! XDG activation tokens are requested by the Wayland thread for action
//! bindings; actual Hyprland activation behavior remains unverified.

use std::io;
use crate::grouping::active_group_ids;
use crate::hooks::NotifyBinding;
use crate::notification::CloseReason;
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
    hit_test_impl(runtime,click,None)
}

/// Hit-test in the coordinate system of one independently rendered Mako
/// surface, not the concatenated legacy preview. Its geometry must match
/// snapshot_surface_frame_scaled for the same identity.
pub fn hit_test_on_surface(
    runtime:&SharedRuntime,click:PointerClick,
    surface_key:&(String,String,String),
)->io::Result<Option<(u32,NotifyBinding)>>{
    hit_test_impl(runtime,click,Some(surface_key))
}

fn hit_test_impl(
    runtime:&SharedRuntime,click:PointerClick,
    surface_key:Option<&(String,String,String)>,
)->io::Result<Option<(u32,NotifyBinding)>>{
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
    let (all_notes,modes)=ordered_visible(runtime)?;
    let notes=all_notes.into_iter().filter(|entry|{
        surface_key.is_none_or(|key|entry.notification.surface_spec.as_ref()==Some(key))
    }).collect::<Vec<_>>();
    // Layer-shell's surface width is the largest notification width.
    // Narrow notifications may be left, centered, or right aligned.
    let mut margins=Vec::with_capacity(notes.len());
    let dimensions:Vec<_>=notes.iter().map(|entry|{
        let n=&entry.notification;
        let style=style_for_note_index(runtime,n,&modes,entry.group_index);
        margins.push(style.get("margin")
            .and_then(|s|PopupMargins::parse_mako(s))
            .unwrap_or(PopupMargins::uniform(10)));
        let w=style.get("width")
            .and_then(|v|crate::config::parse_nonnegative_mako(v))
            .unwrap_or(300) as f64;
        // Hit-testing must use the same Pango shrink-wrapped height as
        // the renderer, not the configured maximum.
        let h=crate::render::measure_notification_height_group(
            n,&style,if entry.group_count>=2{entry.group_count}else{0}
        ).map_err(error)? as f64;
        Ok::<(f64,f64),io::Error>((w,h))
    }).collect::<io::Result<Vec<_>>>()?;
    let anchor=surface_key.map(|key|key.2.as_str()).unwrap_or_else(||
        config.global.get("anchor").map(String::as_str).unwrap_or("top-right"));
    let align=stack_alignment(anchor);
    if dimensions.is_empty(){return Ok(None);}
    let sizes:Vec<_>=dimensions.iter().map(|(w,h)|(*w as u32,*h as u32)).collect();
    let (_,_,positions)=mako_stack_geometry(&sizes,&margins,align).map_err(error)?;
    for ((entry,(w,h)),(x,y)) in notes.iter()
        .zip(dimensions.iter().copied()).zip(positions) {
        let (x,y)=(f64::from(x),f64::from(y));
        let n=&entry.notification;
        let style=style_for_note_index(runtime,n,&modes,entry.group_index);
        if click.y >= y && click.y < y+h {
            // Mako checks the entire rectangle for each notification
            // before moving to the next. In overlapping stacks, a narrow
            // top notification can miss the X coordinate while a wider
            // notification underneath still contains the click.
            if click.x<x || click.x>=x+w {continue;}
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

/// Mako closes all selected group members in one synchronous event-loop
/// turn. Apply state transitions before any .await so a sender cannot replace
/// a still-unclosed ID while the prior NotificationClosed signal is in flight.
/// Keep the established per-ID signal/invalidation sequence afterward.
fn close_selected(
    runtime:&SharedRuntime,ids:impl IntoIterator<Item=u32>,history:bool
)->Result<Vec<u32>,String>{
    let mut removed=Vec::new();
    for id in ids {
        if runtime.close(id,CloseReason::Dismissed,history)? {
            removed.push(id);
        }
    }
    Ok(removed)
}

async fn closed_many(
    conn:&Connection,runtime:&SharedRuntime,ids:impl IntoIterator<Item=u32>,
    history:bool,
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let removed=close_selected(runtime,ids,history).map_err(error)?;
    for id in removed {
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
    let Some((id,binding))=hit_test(runtime,click)? else {return Ok(())};
    dispatch_binding(conn,runtime,id,binding,activation_token).await
}

/// Execute one already-matched Mako binding. This is shared between pointer
/// clicks and the on-notify hook, keeping close reasons and action signals
/// identical in both paths.
pub async fn dispatch_binding(
    conn:&Connection,runtime:&SharedRuntime,id:u32,binding:NotifyBinding,
    activation_token:Option<&str>,
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    match binding {
        NotifyBinding::None=>{}
        NotifyBinding::Exec(cmd)=>NotifyBinding::Exec(cmd).run_exec(id)?,
        NotifyBinding::Dismiss {save_history}=>closed(conn,runtime,id,save_history).await?,
        NotifyBinding::DismissAll=>{
            let ids:Vec<_>=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?
                .active().iter().map(|n|n.id).collect();
            closed_many(conn,runtime,ids,true).await?;
        }
        NotifyBinding::DismissGroup=>{
            let ids=active_group_ids(runtime,id).map_err(error)?;
            closed_many(conn,runtime,ids,true).await?;
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

/// Mako's try_invoke_action() allocates a compositor token as soon as it
/// finds a matching key, BEFORE notify_action_invoked() checks actions=0.
/// Do not conflate token-request eligibility with signal eligibility.
pub fn should_request_activation_token(
    runtime:&SharedRuntime,id:u32,binding:&NotifyBinding,
)->io::Result<bool>{
    let key=match binding {
        NotifyBinding::InvokeDefaultAction=>"default",
        NotifyBinding::InvokeAction(key)=>key.as_str(),
        _=>return Ok(false),
    };
    let store=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?;
    Ok(store.active().iter().find(|n|n.id==id)
        .is_some_and(|n|n.actions.iter().any(|a|a.key==key)))
}

/// Mako's notify_action_invoked() suppresses BOTH action and activation-token
/// signals when the notification's resolved style has actions=0. It still
/// retains the sender action list, and a click still dismisses the popup.
pub fn can_invoke_action(runtime:&SharedRuntime,id:u32,key:&str)->io::Result<bool>{
    let note=runtime.store.lock().map_err(|_|error("notification lock poisoned"))?
        .active().iter().find(|n|n.id==id).cloned();
    let Some(note)=note else{return Ok(false)};
    if !note.actions.iter().any(|action|action.key==key){return Ok(false);}
    let index=crate::grouping::active_group_index(runtime,id).map_err(error)?;
    let modes=runtime.modes().lock().map_err(|_|error("mode lock poisoned"))?
        .clone();
    let style=crate::desktop::style_for_note_index(runtime,&note,&modes,index);
    Ok(style.get("actions")
        .is_none_or(|v|v=="1"||v.eq_ignore_ascii_case("true")))
}

/// Mako invokes a matching sender action and closes its notification
/// synchronously, before returning to its event loop. Do both state
/// operations before any D-Bus signal await, otherwise the sender could
/// replace the same public ID after ActionInvoked but before dismissal.
fn prepare_invocation(
    runtime:&SharedRuntime,id:u32,key:&str,
)->Result<Option<bool>,String>{
    let allowed=can_invoke_action(runtime,id,key).map_err(|e|e.to_string())?;
    if runtime.close(id,CloseReason::Dismissed,true)? {
        Ok(Some(allowed))
    }else{
        Ok(None)
    }
}

async fn invoke(conn:&Connection,runtime:&SharedRuntime,id:u32,key:&str,
    activation_token:Option<&str>)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    // The notification must be removed before yielding for any signal.
    // Signal order still matches Mako: ActivationToken, ActionInvoked,
    // NotificationClosed, then the control properties invalidation.
    let Some(allowed)=prepare_invocation(runtime,id,key).map_err(error)?
        else{return Ok(())};
    if allowed {
        if let Some(token)=activation_token.filter(|token|!token.is_empty()) {
            conn.emit_signal(None::<&str>,OBJECT_PATH,BUS_NAME,
                "ActivationToken",&(id,token)).await?;
        }
        conn.emit_signal(None::<&str>,OBJECT_PATH,BUS_NAME,
            "ActionInvoked",&(id,key)).await?;
    }
    conn.emit_signal(None::<&str>,OBJECT_PATH,BUS_NAME,
        "NotificationClosed",&(id,CloseReason::Dismissed as u32)).await?;
    crate::signals::invalidate(conn,"Notifications").await?;
    Ok(())
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
    fn surface_scoped_hit_testing_cannot_retarget_other_surface(){
        let runtime=SharedRuntime::new(Config::parse(
            "width=110\nheight=32\nborder-size=0\nmargin=0\nmax-visible=1\n\
             [category=left]\nanchor=bottom-left\n"
        ).unwrap());
        let now=std::time::Instant::now();
        let older=runtime.store.lock().unwrap().push(Request{
            summary:"Older root notification".into(),..Request::default()
        });
        runtime.schedule_without_hook(older,now).unwrap();
        let left=runtime.store.lock().unwrap().push(Request{
            category:"left".into(),summary:"Left notification".into(),
            ..Request::default()
        });
        runtime.schedule_without_hook(left,now).unwrap();
        let newest=runtime.store.lock().unwrap().push(Request{
            summary:"Newer root notification".into(),..Request::default()
        });
        runtime.schedule_without_hook(newest,now).unwrap();
        let views=crate::desktop::surface_visibility(&runtime).unwrap();
        let root=&views.iter().find(|v|v.surface_key.2=="top-right").unwrap().surface_key;
        let side=&views.iter().find(|v|v.surface_key.2=="bottom-left").unwrap().surface_key;
        let local=click(10.0,10.0,273);
        assert_eq!(hit_test_on_surface(&runtime,local,root).unwrap().unwrap().0,newest);
        assert_eq!(hit_test_on_surface(&runtime,local,side).unwrap().unwrap().0,left);
        assert_ne!(newest,left);
        assert!(!views.iter().any(|v|v.notification_ids.contains(&older)));
        let unknown=("none".into(),"top".into(),"center".into());
        assert!(hit_test_on_surface(&runtime,local,&unknown).unwrap().is_none());
    }

    #[test]
    fn invoking_action_finishes_old_lifetime_before_any_signal_await() {
        let runtime=SharedRuntime::new(Config::parse("").unwrap());
        let id=runtime.store.lock().unwrap().push(Request{
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],..Request::default()
        });
        let instance=runtime.store.lock().unwrap().active_instance(id).unwrap();
        assert_eq!(prepare_invocation(&runtime,id,"default").unwrap(),Some(true));
        assert!(runtime.store.lock().unwrap().active_instance(id).is_none(),
            "the clicked incarnation must close before ActionInvoked emits");
        let replacement=runtime.store.lock().unwrap().push(Request{
            replaces_id:id,summary:"new message".into(),
            ..Request::default()
        });
        assert_ne!(replacement,id,
            "closed public IDs are not still eligible for replaces_id");
        assert_eq!(runtime.store.lock().unwrap().active_instance(id),None);
        assert_ne!(runtime.store.lock().unwrap().active_instance(replacement),Some(instance));
        assert_eq!(prepare_invocation(&runtime,id,"default").unwrap(),None,
            "stale invoke must never close a newer message");
    }

    #[test]
    fn missing_or_disabled_action_suppresses_signals_but_still_dismisses(){
        let runtime=SharedRuntime::new(Config::parse(
            "[category=disabled]\nactions=0\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request{
            category:"disabled".into(),actions:vec![
                crate::notification::Action{
                    key:"default".into(),label:"Open".into()
                }
            ],..Request::default()
        });
        assert_eq!(prepare_invocation(&runtime,id,"default").unwrap(),Some(false));
        assert!(runtime.store.lock().unwrap().active_instance(id).is_none());
        let other=runtime.store.lock().unwrap().push(Request::default());
        assert_eq!(prepare_invocation(&runtime,other,"default").unwrap(),Some(false),
            "missing sender action must dismiss without ActionInvoked");
    }

    #[test]
    fn batched_group_dismiss_transitions_all_members_before_signalling() {
        let runtime=SharedRuntime::new(Config::parse("group-by=app-name\n").unwrap());
        let first=runtime.store.lock().unwrap().push(Request{
            app_name:"group".into(),summary:"first".into(),
            ..Request::default()
        });
        let outsider=runtime.store.lock().unwrap().push(Request{
            app_name:"other".into(),summary:"independent".into(),
            ..Request::default()
        });
        let second=runtime.store.lock().unwrap().push(Request{
            app_name:"group".into(),summary:"second".into(),
            ..Request::default()
        });
        let members=active_group_ids(&runtime,first).unwrap();
        assert_eq!(members.len(),2);
        assert!(members.contains(&first)&&members.contains(&second));
        let dismissed=close_selected(&runtime,members,true).unwrap();
        assert_eq!(dismissed.len(),2);
        assert!(runtime.store.lock().unwrap().active().iter()
            .all(|n|n.id==outsider),"unrelated notification must survive");
        assert_eq!(runtime.store.lock().unwrap().history().len(),2);
        assert!(runtime.store.lock().unwrap().active_instance(first).is_none());
        assert!(runtime.store.lock().unwrap().active_instance(second).is_none());
        assert!(close_selected(&runtime,[first,second],true).unwrap().is_empty(),
            "repeated close must never emit duplicate NotificationClosed");
    }

    #[test]
    fn overlapping_negative_margins_follow_mako_first_hotspot_rule() {
        let runtime=SharedRuntime::new(Config::parse(
            "width=140\nheight=90\nmargin=0,0,-15,0\n             [category=older]\nmargin=-15,0,0,0\n"
        ).unwrap());
        let older=runtime.store.lock().unwrap().push(Request{
            category:"older".into(),summary:"Older".into(),
            ..Request::default()
        });
        let newer=runtime.store.lock().unwrap().push(Request{
            category:"newer".into(),summary:"Newer".into(),
            ..Request::default()
        });
        let notes=visible_members_indexed(&runtime).unwrap();
        let first=&notes[0];
        let style=style_for_note_index(&runtime,&first.notification,
            &["default".into()],first.group_index);
        let height=crate::render::measure_notification_height_group(
            &first.notification,&style,0
        ).unwrap();
        assert!(height>20);
        // The second popup begins at first.height - 15, so this point
        // overlaps both hotspots. Mako scans its active list first.
        assert_eq!(hit_test(&runtime,click(12.0,f64::from(height)-10.0,273))
            .unwrap().unwrap().0,newer);
        assert_eq!(hit_test(&runtime,click(12.0,f64::from(height)+2.0,273))
            .unwrap().unwrap().0,older);
    }

    #[test]
    fn overlapping_narrow_top_allows_hit_on_wider_notification_below(){
        let runtime=SharedRuntime::new(Config::parse(
            "width=160\nheight=90\nmargin=0,0,-20,0\n             [category=top]\nwidth=80\n             [category=lower]\nmargin=-20,0,0,0\n"
        ).unwrap());
        let lower=runtime.store.lock().unwrap().push(Request{
            category:"lower".into(),summary:"Lower".into(),
            ..Request::default()
        });
        let upper=runtime.store.lock().unwrap().push(Request{
            category:"top".into(),summary:"Upper".into(),
            ..Request::default()
        });
        let notes=visible_members_indexed(&runtime).unwrap();
        assert_eq!(notes[0].notification.id,upper);
        assert_eq!(notes[1].notification.id,lower);
        let upper_style=style_for_note_index(
            &runtime,&notes[0].notification,&["default".into()],notes[0].group_index
        );
        let top_height=crate::render::measure_notification_height_group(
            &notes[0].notification,&upper_style,0
        ).unwrap();
        let y=f64::from(top_height)-10.0;
        // Right aligned: the top (80px) begins at x=80 and misses x=20,
        // while the lower (160px) begins at x=0 and overlaps vertically.
        assert_eq!(hit_test(&runtime,click(20.0,y,273)).unwrap(),
            Some((lower,NotifyBinding::Dismiss{save_history:true})));
        assert_eq!(hit_test(&runtime,click(20.0,y,0)).unwrap(),
            Some((lower,NotifyBinding::Dismiss{save_history:true})),
            "touch-up uses the same Mako first full-hotspot match traversal");
        assert_eq!(hit_test(&runtime,click(100.0,y,273)).unwrap(),
            Some((upper,NotifyBinding::Dismiss{save_history:true})),
            "first matching Mako hotspot keeps precedence");
        assert_eq!(hit_test(&runtime,click(100.0,y,0)).unwrap(),
            Some((upper,NotifyBinding::Dismiss{save_history:true})),
            "touch-up keeps topmost full-hotspot precedence");
    }

    #[test]
    fn whitespace_prefixed_width_matches_renderer_hit_testing(){
        let runtime=SharedRuntime::new(Config::parse(
            "width= 420\nheight=65\nmargin=0\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        assert_eq!(hit_test(&runtime,click(410.0,10.0,273)).unwrap(),
            Some((id,NotifyBinding::Dismiss{save_history:true})));
        assert!(hit_test(&runtime,click(430.0,10.0,273)).unwrap().is_none());
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
    fn actions_style_keeps_keys_but_suppresses_action_signals() {
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
        assert_eq!(runtime.store.lock().unwrap().active().iter()
            .find(|n|n.id==disabled).unwrap().actions.len(),1,
            "Mako stores actions even when signal emission is disabled");
        assert!(can_invoke_action(&runtime,allowed,"default").unwrap());
        assert!(!can_invoke_action(&runtime,allowed,"unknown").unwrap());
    }

    #[test]
    fn mode_switch_rechecks_action_style_for_existing_notification() {
        let runtime=SharedRuntime::new(Config::parse(
            "[category=warning]\nactions=0\n[mode=allow-actions]\nactions=TRUE\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request{
            category:"warning".into(),
            actions:vec![crate::notification::Action{
                key:"default".into(),label:"Open".into(),
            }],..Request::default()
        });
        assert!(!can_invoke_action(&runtime,id,"default").unwrap());
        *runtime.modes().lock().unwrap()=vec!["allow-actions".into()];
        runtime.reapply_active_grouping().unwrap();
        assert!(can_invoke_action(&runtime,id,"default").unwrap(),
            "enabled mode must allow action signals without replacing the note");
        *runtime.modes().lock().unwrap()=vec!["default".into()];
        runtime.reapply_active_grouping().unwrap();
        assert!(!can_invoke_action(&runtime,id,"default").unwrap());
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
    fn grouped_actions_style_disables_signals_after_grouping() {
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
        let now=std::time::Instant::now();
        let first=runtime.store.lock().unwrap().push(request());
        runtime.schedule_without_hook(first,now).unwrap();
        assert!(can_invoke_action(&runtime,first,"default").unwrap());
        let second=runtime.store.lock().unwrap().push(request());
        runtime.schedule_without_hook(second,now).unwrap();
        assert!(!can_invoke_action(&runtime,first,"default").unwrap());
        assert!(!can_invoke_action(&runtime,second,"default").unwrap());
    }

    #[test]
    fn activation_token_requests_follow_live_action_style() {
        let runtime=SharedRuntime::new(Config::parse(
            "[category=warning]\nactions=FALSE\n"
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
        let invoke=NotifyBinding::InvokeDefaultAction;
        assert!(needs_activation_token(&invoke),
            "binding kind may need activation when style permits");
        // Upstream requests the compositor token before actions=0 prevents
        // ActionInvoked/ActivationToken signals.
        assert!(should_request_activation_token(&runtime,disabled,&invoke).unwrap());
        assert!(!can_invoke_action(&runtime,disabled,"default").unwrap());
        assert!(should_request_activation_token(&runtime,allowed,&invoke).unwrap());
        assert!(!should_request_activation_token(&runtime,allowed,
            &NotifyBinding::InvokeAction("missing".into())).unwrap());
        assert!(!should_request_activation_token(&runtime,allowed,
            &NotifyBinding::Dismiss{save_history:true}).unwrap());
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
