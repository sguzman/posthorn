//! An opt-in, continuous Wayland display driven by Posthorn's private
//! D-Bus test service. Never requests org.freedesktop.Notifications.
//! The renderer uses one stacked layer-shell surface and deliberately has no
//! input handling or full Mako output-selection semantics yet.

use std::io;
use std::time::Duration;
use crate::config::{Options, Traits};
use crate::grouping::{GroupSpec,group_ordered};
use crate::notification::{Notification, Urgency};
use crate::render::{compose_stack_spaced, render_notification_group, render_hidden_placeholder, PixelFrame,StackAlignment};
use crate::runtime::SharedRuntime;
use crate::wayland::{LivePresenter, ScreenCorner,PopupLayer,PopupMargins,PointerClick};

pub fn stack_alignment(anchor:&str)->StackAlignment {
    if anchor.ends_with("-right"){StackAlignment::Right}
    else if anchor.ends_with("-left"){StackAlignment::Left}
    else{StackAlignment::Center}
}

/// Extract per-notification directional margins; each adjacent vertical
/// spacer is the max of the previous bottom and next top margin.
pub fn vertical_spacing(margins:&[PopupMargins])->Vec<u32>{
    margins.windows(2).map(|pair|{
        pair[0].bottom.max(pair[1].top).max(0) as u32
    }).collect()
}

fn poisoned() -> io::Error { io::Error::other("notification state lock poisoned") }
fn other(e: impl std::fmt::Display) -> io::Error { io::Error::other(e.to_string()) }

/// Resolve style for the same notification fields in the ordinary runtime.
/// All four Mako category colors are evaluated independently.
pub fn style_for_note(
    runtime:&SharedRuntime,n:&Notification,modes:&[String],
)->Options {
    style_for_note_group(runtime,n,modes,1)
}

pub fn style_for_note_group(
    runtime:&SharedRuntime,n:&Notification,modes:&[String],group_count:usize,
)->Options {
    let config=runtime.config();
    let mode_views: Vec<&str>=modes.iter().map(String::as_str).collect();
    let traits=Traits {
        app_name:&n.app_name, app_icon:&n.app_icon, category:&n.category,
        desktop_entry:&n.desktop_entry, summary:&n.summary, body:&n.body,
        urgency:match n.urgency {
            Urgency::Low=>"low",Urgency::Normal=>"normal",Urgency::Critical=>"critical"
        },
        anchor:config.global.get("anchor")
            .map(String::as_str).unwrap_or("top-right"),
        output:config.global.get("output").map(String::as_str).unwrap_or(""),
        modes:&mode_views,
        requested_timeout_ms:n.requested_timeout_ms,
        actionable:!n.actions.is_empty(),
        group_index:if group_count>=2{0}else{-1},hidden:false,
    };
    config.style_for(&traits)
}

/// Select only the first visible member from each Mako grouping pass.
/// The hidden-count placeholder is separate TODO work; this prevents
/// duplicate group popups and applies max-visible to groups rather than IDs.
pub fn visible_notifications(runtime:&SharedRuntime)->io::Result<Vec<Notification>>{
    Ok(visible_members(runtime)?.into_iter().map(|(note,_)|note).collect())
}

/// Return the count of each displayed notification's group as well as its ID.
/// Keeping this together prevents formatting and mouse geometry from diverging.
pub fn visible_members(runtime:&SharedRuntime)
    -> io::Result<Vec<(Notification,usize)>>
{
    let config=runtime.config();
    let limit=config.global.get("max-visible")
        .and_then(|v|v.parse::<i32>().ok()).unwrap_or(5);
    // Upstream Mako treats negative max-visible as unlimited.
    let max_visible=if limit<0 {usize::MAX} else {limit as usize};
    let spec=config.global.get("group-by")
        .map(String::as_str).unwrap_or("none");
    let spec=GroupSpec::parse(spec).map_err(other)?;
    // Keep lock acquisition order consistent with SharedRuntime::schedule:
    // store first, modes second. The reverse order can deadlock with a
    // concurrent Notify() while the popup thread is reading groups.
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let by_id=store.active();
    let mut selected=Vec::new();
    for group in group_ordered(by_id,&spec) {
        if let Some(id)=group.ids.first(){
            if let Some(n)=by_id.iter().find(|n|&n.id==id){
                let count=group.ids.len();
                let style=style_for_note_group(runtime,n,&modes,count);
                if style.get("invisible").is_some_and(|v|v=="1"||v=="true"){
                    continue;
                }
                if selected.len()>=max_visible {break;}
                selected.push((n.clone(),count));
            }
        }
    }
    Ok(selected)
}

/// Count real groups eligible for display, excluding explicitly invisible
/// categories. This excludes them from synthetic hidden-count placeholders.
pub fn displayable_groups(runtime:&SharedRuntime)->io::Result<usize>{
    let config=runtime.config();
    let group_spec=config.global.get("group-by")
        .map(String::as_str).unwrap_or("none");
    let spec=GroupSpec::parse(group_spec).map_err(other)?;
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let mut count=0usize;
    for group in group_ordered(store.active(),&spec){
        if let Some(id)=group.ids.first(){
            if let Some(n)=store.active().iter().find(|n|&n.id==id){
                let style=style_for_note_group(runtime,n,&modes,group.ids.len());
                if !style.get("invisible").is_some_and(|v|v=="1"||v=="true"){
                    count+=1;
                }
            }
        }
    }
    Ok(count)
}

/// Render the active, ordered messages at the present instant, including
/// configured category colors. Returns None for an empty notification list.
pub fn snapshot_frame(runtime:&SharedRuntime) -> io::Result<Option<PixelFrame>> {
    let config=runtime.config();
    let notes=visible_members(runtime)?;
    let displayable=displayable_groups(runtime)?;
    let total=runtime.store.lock().map_err(|_|poisoned())?.active().len();
    let hidden=displayable.saturating_sub(notes.len());
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let mut frames=Vec::with_capacity(notes.len());
    let mut margins=Vec::with_capacity(notes.len());
    for (note,group_count) in &notes {
        let style=style_for_note_group(runtime,note,&modes,*group_count);
        margins.push(style.get("margin")
            .and_then(|s|PopupMargins::parse_mako(s))
            .unwrap_or(PopupMargins::uniform(10)));
        frames.push(render_notification_group(
            note,&style,if *group_count>=2{*group_count}else{0}
        ).map_err(other)?);
    }
    if hidden>0 {
        let mode_views:Vec<&str>=modes.iter().map(String::as_str).collect();
        let traits=Traits{
            app_name:"",app_icon:"",category:"",desktop_entry:"",
            summary:"",body:"",urgency:"normal",
            anchor:config.global.get("anchor")
                .map(String::as_str).unwrap_or("top-right"),
            output:config.global.get("output").map(String::as_str).unwrap_or(""),
            modes:&mode_views,requested_timeout_ms:-1,
            actionable:false,group_index:-1,hidden:true,
        };
        let style=config.style_for(&traits);
        if !style.get("invisible").is_some_and(|x|x=="1"||x=="true"){
            frames.push(render_hidden_placeholder(hidden,total,&style).map_err(other)?);
            margins.push(style.get("margin")
                .and_then(|s|PopupMargins::parse_mako(s))
                .unwrap_or(PopupMargins::uniform(10)));
        }
    }
    let align=stack_alignment(config.global.get("anchor")
        .map(String::as_str).unwrap_or("top-right"));
    compose_stack_spaced(&frames,&vertical_spacing(&margins),align).map_err(other)
}

fn render_loop(runtime:SharedRuntime,sender:tokio::sync::mpsc::UnboundedSender<PointerClick>) -> io::Result<()> {
    let mut viewer=LivePresenter::connect().map_err(other)?;
    let (mut previous, mut previous_modes)=(Vec::new(),Vec::new());
    let mut previous_revision=u64::MAX;
    loop {
        let notes: Vec<_>=runtime.store.lock().map_err(|_|poisoned())?
            .active().iter().cloned().collect();
        let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
        let revision=runtime.revision();
        if notes!=previous || modes!=previous_modes || revision!=previous_revision {
            let config=runtime.config();
            let frame=snapshot_frame(&runtime)?;
            let corner=config.global.get("anchor")
                .and_then(|s|ScreenCorner::from_mako(s))
                .unwrap_or(ScreenCorner::TopRight);
            let margins=config.global.get("margin")
                .and_then(|s|PopupMargins::parse_mako(s))
                .unwrap_or(PopupMargins::uniform(10));
            let layer=config.global.get("layer")
                .and_then(|s|PopupLayer::from_mako(s))
                .unwrap_or(PopupLayer::Top);
            let output=config.global.get("output").map(String::as_str).unwrap_or("");
            viewer.update_frame_on_output(frame.as_ref(),corner,margins,layer,output)
                .map_err(other)?;
            previous=notes;
            previous_modes=modes;
            previous_revision=revision;
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
    run_private_desktop_instance(conn,runtime).await
}

pub async fn run_private_desktop_from_path(path:&std::path::Path)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    let (conn,runtime)=crate::service::open_private_service_from_path(path).await?;
    run_private_desktop_instance(conn,runtime).await
}

async fn run_private_desktop_instance(conn:zbus::Connection,runtime:SharedRuntime)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
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
        assert_eq!(pixels.width,100);
        assert!(pixels.height<84);
        let first_height=(pixels.height-4)/2;
        let idx=((first_height/2) as usize*pixels.stride as usize)+90*4;
        assert_eq!(&pixels.pixels[idx..idx+4],&[0x48,0x41,0x68,0xff]);
        let idx=((first_height+4+first_height/2) as usize*pixels.stride as usize)+90*4;
        assert_eq!(&pixels.pixels[idx..idx+4],&[0x47,0x57,0x30,0xff]);
        runtime.close(a,CloseReason::Dismissed,false).unwrap();
        let after=snapshot_frame(&runtime).unwrap().unwrap();
        assert_eq!(after.height,first_height);
    }

    #[test]
    fn style_matching_uses_the_configured_anchor_instead_of_top_right() {
        let runtime=SharedRuntime::new(Config::parse(
            "anchor=bottom-left\n             [anchor=bottom-left]\nbackground-color=#715536\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let note=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        let mode=vec!["default".to_string()];
        let style=style_for_note(&runtime,&note,&mode);
        assert_eq!(style.get("background-color").map(String::as_str),Some("#715536"));
    }

    #[test]
    fn explicit_output_name_affects_mako_style_selection() {
        let runtime=SharedRuntime::new(Config::parse(
            "output=DP-2\n[output=DP-2]\nbackground-color=#305747\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let note=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        let modes=["default".to_string()];
        let style=style_for_note(&runtime,&note,&modes);
        assert_eq!(style.get("background-color").map(String::as_str),
            Some("#305747"));
    }

    #[test]
    fn adjacent_mako_margins_use_the_larger_edge() {
        let margins=[
            PopupMargins{top:1,right:0,bottom:9,left:0},
            PopupMargins{top:4,right:0,bottom:2,left:0},
            PopupMargins{top:7,right:0,bottom:3,left:0},
        ];
        assert_eq!(vertical_spacing(&margins),vec![9,7]);
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
    fn grouped_first_notification_has_correct_count_format_and_metadata() {
        let runtime=SharedRuntime::new(Config::parse("group-by=app-name\n").unwrap());
        for _ in 0..3 {
            runtime.store.lock().unwrap().push(Request{
                app_name:"one".into(),summary:"Task".into(),..Request::default()
            });
        }
        let members=visible_members(&runtime).unwrap();
        assert_eq!(members.len(),1);
        assert_eq!(members[0].1,3);
        let style=style_for_note_group(&runtime,&members[0].0,&["default".into()],3);
        assert_eq!(style.get("format").map(String::as_str),
            Some("(%g) <b>%s</b>\\n%b"));
        let frame=snapshot_frame(&runtime).unwrap().unwrap();
        assert!(frame.height>0);
    }

    #[test]
    fn invisible_notifications_do_not_consume_visible_slots_or_hidden_count() {
        let runtime=SharedRuntime::new(Config::parse(
            "max-visible=1\n[category=silent]\ninvisible=1\n"
        ).unwrap());
        let old=runtime.store.lock().unwrap().push(Request{
            category:"ordinary".into(),..Request::default()
        });
        runtime.store.lock().unwrap().push(Request{
            category:"silent".into(),..Request::default()
        });
        let newest=runtime.store.lock().unwrap().push(Request{
            category:"ordinary".into(),..Request::default()
        });
        let shown=visible_notifications(&runtime).unwrap();
        assert_eq!(shown.len(),1);
        assert_eq!(shown[0].id,newest);
        assert_eq!(displayable_groups(&runtime).unwrap(),2);
        assert_ne!(shown[0].id,old);
        assert_eq!(runtime.store.lock().unwrap().active().len(),3);
    }

    #[test]
    fn max_visible_adds_a_hidden_placeholder_without_mutating_store() {
        let runtime=SharedRuntime::new(Config::parse(
            "max-visible=1\nwidth=140\nheight=70\nmargin=4\n"
        ).unwrap());
        for id in 0..3 {
            runtime.store.lock().unwrap().push(Request{
                summary:format!("message-{id}"),..Request::default()
            });
        }
        let first=visible_notifications(&runtime).unwrap();
        assert_eq!(first.len(),1);
        let frame=snapshot_frame(&runtime).unwrap().unwrap();
        let styles=style_for_note(&runtime,&first[0],&["default".into()]);
        let first_height=crate::render::measure_notification_height(&first[0],&styles).unwrap();
        assert!(frame.height>first_height+4,"hidden placeholder must be visible");
        assert_eq!(runtime.store.lock().unwrap().active().len(),3);
        assert!(runtime.store.lock().unwrap().history().is_empty());
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
