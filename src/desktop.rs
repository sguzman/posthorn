//! An opt-in, continuous Wayland display driven by Posthorn's private
//! D-Bus test service. Never requests org.freedesktop.Notifications.
//! The renderer uses one stacked layer-shell surface, input bindings and
//! integer output scaling; multi-output and native HiDPI QA remain.

use std::io;
use std::time::Duration;
use crate::config::{Options, Traits};
use crate::grouping::{GroupSpec,group_ordered};
use crate::notification::{Notification, Urgency};
use crate::render::{compose_mako_stack, render_notification_group, render_hidden_placeholder, PixelFrame,StackAlignment};
use crate::runtime::SharedRuntime;
use crate::wayland::{LivePresenter, ScreenCorner,PopupLayer,PopupMargins,PointerClick};

/// Mako applies outer-margin to the layer-shell window; margin belongs to
/// the per-notification layout and must never shift the entire surface.
pub fn outer_margins(config:&crate::config::Config)->PopupMargins {
    config.global.get("outer-margin")
        .and_then(|s|PopupMargins::parse_mako(s))
        .unwrap_or(PopupMargins::uniform(0))
}

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
    style_for_note_index(runtime,n,modes,if group_count>=2{0}else{-1})
}

pub fn style_for_note_index(
    runtime:&SharedRuntime,n:&Notification,modes:&[String],group_index:i32,
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
        group_index,hidden:false,
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
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct VisibleMember {
    pub notification:Notification,
    pub group_count:usize,
    pub group_index:i32,
}

pub fn visible_members(runtime:&SharedRuntime)
    -> io::Result<Vec<(Notification,usize)>>
{
    Ok(visible_members_indexed(runtime)?.into_iter()
        .map(|m|(m.notification,m.group_count)).collect())
}

#[derive(Debug)]
struct VisibleProjection {
    members:Vec<VisibleMember>,
    hidden_count:usize,
    total_count:usize,
}

pub fn visible_members_indexed(runtime:&SharedRuntime)
    -> io::Result<Vec<VisibleMember>>
{
    Ok(visible_projection(runtime)?.members)
}

fn visible_projection(runtime:&SharedRuntime)->io::Result<VisibleProjection>{
    let config=runtime.config();
    let spec=config.global.get("group-by")
        .map(String::as_str).unwrap_or("none");
    let spec=GroupSpec::parse(spec).map_err(other)?;
    // Keep lock acquisition order consistent with SharedRuntime::schedule:
    // store first, modes second. The reverse order can deadlock with a
    // concurrent Notify() while the popup thread is reading groups.
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let by_id=store.active();
    let total_count=by_id.len();
    let mut selected=Vec::new();
    let mut hidden_count=0usize;
    let mut displayed_groups=0usize;
    for group in group_ordered(by_id,&spec) {
        let count=group.ids.len();
        for (index,id) in group.ids.iter().enumerate() {
            if let Some(note)=by_id.iter().find(|n|&n.id==id) {
                let group_index=if count>1{index as i32}else{-1};
                let style=style_for_note_index(runtime,note,&modes,group_index);
                let limit=style.get("max-visible")
                    .and_then(|v|v.parse::<i32>().ok()).unwrap_or(5);
                if limit>=0&&displayed_groups>=limit as usize {
                    // Upstream increments hidden count for every excluded
                    // notification, including secondary grouped members.
                    hidden_count+=1;
                    continue;
                }
                if style.get("invisible").is_some_and(|v|
                    v=="1"||v.eq_ignore_ascii_case("true")) {
                    continue;
                }
                selected.push(VisibleMember{
                    notification:note.clone(),group_count:count,group_index,
                });
                if group_index<=0 {displayed_groups+=1;}
            }
        }
    }
    Ok(VisibleProjection{
        members:selected,hidden_count,total_count,
    })
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
    let projection=visible_projection(runtime)?;
    let notes=projection.members;
    let total=projection.total_count;
    let hidden=projection.hidden_count;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let mut frames=Vec::with_capacity(notes.len());
    let mut margins=Vec::with_capacity(notes.len());
    for entry in &notes {
        let note=&entry.notification;
        let style=style_for_note_index(runtime,note,&modes,entry.group_index);
        margins.push(style.get("margin")
            .and_then(|s|PopupMargins::parse_mako(s))
            .unwrap_or(PopupMargins::uniform(10)));
        frames.push(render_notification_group(
            note,&style,if entry.group_count>=2{entry.group_count}else{0}
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
    compose_mako_stack(&frames,&margins,align).map_err(other)
}

struct DesktopClick {
    click:PointerClick,
    activation_token:Option<String>,
}

fn render_loop(runtime:SharedRuntime,sender:tokio::sync::mpsc::UnboundedSender<DesktopClick>) -> io::Result<()> {
    let mut viewer=LivePresenter::connect().map_err(other)?;
    let (mut previous, mut previous_modes)=(Vec::new(),Vec::new());
    let mut previous_revision=u64::MAX;
    let mut previous_scale=0u32;
    loop {
        let notes: Vec<_>=runtime.store.lock().map_err(|_|poisoned())?
            .active().iter().cloned().collect();
        let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
        // Read Wayland output scale events before deciding whether a fresh
        // logical image must be committed with new physical SHM dimensions.
        viewer.poll_events().map_err(other)?;
        let config=runtime.config();
        let selected_output=config.global.get("output")
            .map(String::as_str).unwrap_or("");
        let scale=viewer.output_scale(selected_output);
        let revision=runtime.revision();
        if notes!=previous || modes!=previous_modes ||
            revision!=previous_revision || scale!=previous_scale {

            let frame=snapshot_frame(&runtime)?;
            let corner=config.global.get("anchor")
                .and_then(|s|ScreenCorner::from_mako(s))
                .unwrap_or(ScreenCorner::TopRight);
            let margins=outer_margins(&config);
            let layer=config.global.get("layer")
                .and_then(|s|PopupLayer::from_mako(s))
                .unwrap_or(PopupLayer::Top);
            let output=config.global.get("output").map(String::as_str).unwrap_or("");
            if frame.is_none()||!viewer.buffer_backpressure(){
                viewer.update_frame_on_output(frame.as_ref(),corner,margins,layer,output)
                    .map_err(other)?;
                previous=notes;
                previous_modes=modes;
                previous_revision=revision;
                previous_scale=scale;
            }
            // Otherwise leave the previous snapshots unchanged: the next
            // compositor buffer release retries the newest frame, without
            // dropping notification or input events.
        }
        for click in viewer.take_clicks() {
            // Request activation only for action bindings. Ordinary dismiss
            // clicks must not incur a compositor token roundtrip.
            let invokes_action=crate::input::hit_test(&runtime,click)
                .map_err(other)?
                .as_ref()
                .is_some_and(|(_,binding)|
                    crate::input::needs_activation_token(binding));
            let activation_token=if invokes_action {
                viewer.request_activation_token(click.serial).map_err(other)?
            }else{None};
            if sender.send(DesktopClick{click,activation_token}).is_err() {
                return Ok(());
            }
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
                if let Some(event)=click {
                    crate::input::dispatch_click_with_activation(
                        &conn,&runtime,event.click,event.activation_token.as_deref()
                    ).await?;
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
        assert_eq!(pixels.width,108);
        assert!(pixels.height<88);
        let first_height=(pixels.height-12)/2;
        let idx=((4+first_height/2) as usize*pixels.stride as usize)+94*4;
        assert_eq!(&pixels.pixels[idx..idx+4],&[0x48,0x41,0x68,0xff]);
        let idx=((8+first_height+first_height/2) as usize*pixels.stride as usize)+94*4;
        assert_eq!(&pixels.pixels[idx..idx+4],&[0x47,0x57,0x30,0xff]);
        runtime.close(a,CloseReason::Dismissed,false).unwrap();
        let after=snapshot_frame(&runtime).unwrap().unwrap();
        let remaining=runtime.store.lock().unwrap().active()[0].clone();
        let style=style_for_note(&runtime,&remaining,&["default".into()]);
        let measured=crate::render::measure_notification_height(&remaining,&style).unwrap();
        assert_eq!(after.height,measured+8,
            "the remaining popup keeps both its top and trailing bottom margin");
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
    fn layer_shell_window_uses_outer_margin_not_notification_margin() {
        let default=Config::parse("margin=18\n").unwrap();
        assert_eq!(outer_margins(&default),PopupMargins::uniform(0));
        let explicit=Config::parse(
            "margin=18\nouter-margin=3,5,7,9\n"
        ).unwrap();
        assert_eq!(outer_margins(&explicit),PopupMargins{
            top:3,right:5,bottom:7,left:9,
        });
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
    fn per_category_max_visible_controls_own_notification_selection(){
        let runtime=SharedRuntime::new(Config::parse(
            "max-visible=1\n[category=urgent]\nmax-visible=3\n"
        ).unwrap());
        // Newest first: the third slot is permitted by the category rule.
        let older=runtime.store.lock().unwrap().push(Request{
            category:"urgent".into(),..Request::default()
        });
        let middle=runtime.store.lock().unwrap().push(Request{
            category:"urgent".into(),..Request::default()
        });
        let newest=runtime.store.lock().unwrap().push(Request::default());
        let shown=visible_notifications(&runtime).unwrap();
        assert_eq!(shown.iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![newest,middle,older]);
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
    fn grouped_secondary_member_can_be_made_visible_by_user_rule() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\nmax-visible=4\n[grouped]\ninvisible=0\n"
        ).unwrap());
        let old=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        let new=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        let visible=visible_members_indexed(&runtime).unwrap();
        assert_eq!(visible.iter().map(|v|(v.notification.id,v.group_index))
            .collect::<Vec<_>>(),vec![(new,0),(old,1)]);
        assert!(snapshot_frame(&runtime).unwrap().is_some());
        let defaults=SharedRuntime::new(Config::parse("group-by=app-name\n").unwrap());
        for _ in 0..2 {
            defaults.store.lock().unwrap().push(Request{
                app_name:"same".into(),..Request::default()
            });
        }
        assert_eq!(visible_members_indexed(&defaults).unwrap().len(),1);
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
    fn hidden_count_tracks_notifications_not_only_overflowing_groups(){
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=app-name\nmax-visible=1\n"
        ).unwrap());
        runtime.store.lock().unwrap().push(Request{
            app_name:"other".into(),..Request::default()
        });
        runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),..Request::default()
        });
        let projection=visible_projection(&runtime).unwrap();
        assert_eq!(projection.members.len(),1);
        assert_eq!(projection.hidden_count,2,
            "overflow counts the secondary group member and next group");
        assert_eq!(projection.total_count,3);
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
