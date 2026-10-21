//! An opt-in, continuous Wayland display driven by Posthorn's private
//! D-Bus test service. Never requests org.freedesktop.Notifications.
//! The renderer uses one stacked layer-shell surface and deliberately has no
//! input handling or full Mako output-selection semantics yet.

use std::io;
use std::time::Duration;
use crate::config::{Options, Traits};
use crate::grouping::{GroupSpec,group_ordered};
use crate::notification::{Notification, Urgency};
use crate::render::{compose_stack_aligned, render_notification, PixelFrame,StackAlignment};
use crate::runtime::SharedRuntime;
use crate::wayland::{LivePresenter, ScreenCorner,PopupLayer,PopupMargins,PointerClick};

pub fn stack_alignment(anchor:&str)->StackAlignment {
    if anchor.ends_with("-right"){StackAlignment::Right}
    else if anchor.ends_with("-left"){StackAlignment::Left}
    else{StackAlignment::Center}
}

fn poisoned() -> io::Error { io::Error::other("notification state lock poisoned") }
fn other(e: impl std::fmt::Display) -> io::Error { io::Error::other(e.to_string()) }

/// Resolve style for the same notification fields in the ordinary runtime.
/// All four Mako category colors are evaluated independently.
pub fn style_for_note(
    runtime: &SharedRuntime, n: &Notification, modes: &[String],
) -> Options {
    let mode_views: Vec<&str>=modes.iter().map(String::as_str).collect();
    let traits=Traits {
        app_name:&n.app_name, app_icon:&n.app_icon, category:&n.category,
        desktop_entry:&n.desktop_entry, summary:&n.summary, body:&n.body,
        urgency:match n.urgency {
            Urgency::Low=>"low",Urgency::Normal=>"normal",Urgency::Critical=>"critical"
        },
        anchor:"top-right",output:"",modes:&mode_views,
        requested_timeout_ms:n.requested_timeout_ms,
        actionable:!n.actions.is_empty(),group_index:-1,hidden:false,
    };
    runtime.config().style_for(&traits)
}

/// Select only the first visible member from each Mako grouping pass.
/// The hidden-count placeholder is separate TODO work; this prevents
/// duplicate group popups and applies max-visible to groups rather than IDs.
pub fn visible_notifications(runtime:&SharedRuntime) -> io::Result<Vec<Notification>> {
    let limit=runtime.config().global.get("max-visible")
        .and_then(|v|v.parse::<i32>().ok()).unwrap_or(5);
    // Upstream Mako treats negative max-visible as unlimited.
    let max_visible=if limit<0 {usize::MAX} else {limit as usize};
    let spec=runtime.config().global.get("group-by")
        .map(String::as_str).unwrap_or("none");
    let spec=GroupSpec::parse(spec).map_err(other)?;
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    let by_id=store.active();
    let mut selected=Vec::new();
    for group in group_ordered(by_id,&spec).into_iter().take(max_visible) {
        if let Some(id)=group.ids.first(){
            if let Some(n)=by_id.iter().find(|n|&n.id==id){
                selected.push(n.clone());
            }
        }
    }
    Ok(selected)
}

/// Render the active, ordered messages at the present instant, including
/// configured category colors. Returns None for an empty notification list.
pub fn snapshot_frame(runtime:&SharedRuntime) -> io::Result<Option<PixelFrame>> {
    let notes=visible_notifications(runtime)?;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let mut frames=Vec::with_capacity(notes.len());
    for note in &notes {
        let style=style_for_note(runtime,note,&modes);
        frames.push(render_notification(note,&style).map_err(other)?);
    }
    let gap=runtime.config().global.get("margin")
        .and_then(|s|s.split(',').next()).and_then(|s|s.parse::<u32>().ok())
        .unwrap_or(10);
    let align=stack_alignment(runtime.config().global.get("anchor")
        .map(String::as_str).unwrap_or("top-right"));
    compose_stack_aligned(&frames,gap,align).map_err(other)
}

fn render_loop(runtime:SharedRuntime,sender:tokio::sync::mpsc::UnboundedSender<PointerClick>) -> io::Result<()> {
    let mut viewer=LivePresenter::connect().map_err(other)?;
    let (mut previous, mut previous_modes)=(Vec::new(),Vec::new());
    loop {
        let notes: Vec<_>=runtime.store.lock().map_err(|_|poisoned())?
            .active().iter().cloned().collect();
        let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
        if notes!=previous || modes!=previous_modes {
            let frame=snapshot_frame(&runtime)?;
            let corner=runtime.config().global.get("anchor")
                .and_then(|s|ScreenCorner::from_mako(s))
                .unwrap_or(ScreenCorner::TopRight);
            let margins=runtime.config().global.get("margin")
                .and_then(|s|PopupMargins::parse_mako(s))
                .unwrap_or(PopupMargins::uniform(10));
            let layer=runtime.config().global.get("layer")
                .and_then(|s|PopupLayer::from_mako(s))
                .unwrap_or(PopupLayer::Top);
            viewer.update_frame_with_options(frame.as_ref(),corner,margins,layer)
                .map_err(other)?;
            previous=notes;
            previous_modes=modes;
        }
        viewer.poll_events().map_err(other)?;
        for click in viewer.take_clicks() {
            // No GUI thread can directly mutate D-Bus state or emit signals.
            if sender.send(click).is_err() { return Ok(()); }
        }
        std::thread::sleep(Duration::from_millis(45));
    }
}

/// Drive the D-Bus service and the live Wayland preview in one process, each
/// with its own event-loop thread. Both components share notification state.
pub async fn run_private_desktop(config: crate::config::Config)
    -> Result<(),Box<dyn std::error::Error + Send + Sync>>
{
    let (conn,runtime)=crate::service::open_private_service(config).await?;
    let visual_runtime=runtime.clone();
    let (sender,mut clicks)=tokio::sync::mpsc::unbounded_channel();
    let mut visual=tokio::task::spawn_blocking(move || render_loop(visual_runtime,sender));
    let mut timer=tokio::time::interval(Duration::from_millis(25));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    eprintln!(
        "Posthorn private desktop preview: bus org.posthorn.Development only; Mako remains active"
    );
    loop {
        tokio::select! {
            click=clicks.recv() => {
                if let Some(click)=click {
                    crate::input::dispatch_click(&conn,&runtime,click).await?;
                }
            }
            result = &mut visual => {
                return match result {
                    Ok(Ok(()))=>Ok(()),
                    Ok(Err(err))=>Err(Box::new(err)),
                    Err(err)=>Err(Box::new(err)),
                };
            }
            _ = timer.tick() => {
                crate::service::tick_expirations(&conn,&runtime).await?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::notification::{Request,CloseReason};
    use std::time::Instant;

    #[test]
    fn headless_private_desktop_snapshot_shows_two_category_colors() {
        let cfg=Config::parse(
            "width=100\nheight=40\nborder-size=0\nmargin=4\nmax-visible=2\n             background-color=#3B4858\n             [category=success]\nbackground-color=#305747\n             [category=failure]\nbackground-color=#684148\n"
        ).unwrap();
        let runtime=SharedRuntime::new(cfg);
        let a=runtime.store.lock().unwrap().push(Request{
            summary:"alpha".into(),category:"success".into(),..Request::default()
        });
        runtime.store.lock().unwrap().push(Request{
            summary:"beta".into(),category:"failure".into(),..Request::default()
        });
        let pixels=snapshot_frame(&runtime).unwrap().unwrap();
        assert_eq!((pixels.width,pixels.height),(100,84));
        let idx=(30*pixels.stride as usize)+90*4;
        assert_eq!(&pixels.pixels[idx..idx+4],&[0x48,0x41,0x68,0xff]);
        let idx=(74*pixels.stride as usize)+90*4;
        assert_eq!(&pixels.pixels[idx..idx+4],&[0x47,0x57,0x30,0xff]);
        runtime.close(a,CloseReason::Dismissed,false).unwrap();
        let after=snapshot_frame(&runtime).unwrap().unwrap();
        assert_eq!(after.height,40);
    }

    #[test]
    fn negative_max_visible_keeps_all_groups_in_display_order() {
        let runtime=SharedRuntime::new(Config::parse("max-visible=-1\n").unwrap());
        for _ in 0..7 {
            runtime.store.lock().unwrap().push(Request::default());
        }
        assert_eq!(visible_notifications(&runtime).unwrap().len(),7);
    }

    #[test]
    fn grouping_hides_secondary_members_but_keeps_distinct_apps_visible() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\nmax-visible=2\n"
        ).unwrap());
        let mut store=runtime.store.lock().unwrap();
        let older=store.push(Request{app_name:"alpha".into(),..Request::default()});
        let separate=store.push(Request{app_name:"beta".into(),..Request::default()});
        let newer=store.push(Request{app_name:"alpha".into(),..Request::default()});
        drop(store);
        let visible=visible_notifications(&runtime).unwrap();
        assert_eq!(visible.len(),2);
        assert_eq!(visible.iter().map(|n|n.id).collect::<Vec<_>>(),vec![newer,separate]);
        assert!(!visible.iter().any(|n|n.id==older));
    }

    #[test]
    fn empty_snapshot_has_no_surface() {
        let runtime=SharedRuntime::new(Config::default());
        assert!(snapshot_frame(&runtime).unwrap().is_none());
    }
}
