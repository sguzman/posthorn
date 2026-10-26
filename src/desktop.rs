//! An opt-in, continuous Wayland display driven by Posthorn's private
//! D-Bus test service. Never requests org.freedesktop.Notifications.
//! The renderer uses one stacked layer-shell surface, input bindings and
//! native-resolution integer scaling; fractional scale and live Hyprland QA remain.

use std::io;
use std::time::Duration;
use crate::config::{Options, Traits};
use crate::grouping::group_ordered_scoped;
use crate::notification::Notification;
use crate::render::{compose_mako_stack,compose_mako_stack_with_visible_count,mako_stack_geometry,render_notification_group_with_icon_scale,render_notification_group_with_output_subpixel,render_hidden_placeholder_scaled,render_hidden_placeholder_with_subpixel,OutputSubpixel,PixelFrame,StackAlignment};
use crate::runtime::SharedRuntime;
use crate::wayland::{LivePresenter, ScreenCorner,PopupLayer,PopupMargins};

/// Pinned Mako v1.11.0 applies `config.superstyle.outer_margin` to
/// every layer-shell window. config.c:apply_superset_style takes the signed,
/// per-edge maximum across the root and ALL criteria, regardless of which
/// criteria match the current notification. Its target starts with zeroes.
/// Notification `margin` remains a separate, per-notification inset.
pub fn outer_margins(config:&crate::config::Config)->PopupMargins {
    let mut result=PopupMargins::uniform(0);
    let mut include=|raw:Option<&String>|{
        if let Some(other)=raw.and_then(|s|PopupMargins::parse_mako(s)){
            result.top=result.top.max(other.top);
            result.right=result.right.max(other.right);
            result.bottom=result.bottom.max(other.bottom);
            result.left=result.left.max(other.left);
        }
    };
    include(config.global.get("outer-margin"));
    for rule in &config.rules {
        include(rule.options.get("outer-margin"));
    }
    result
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
    let observed_output=runtime.surface_output();
    let traits=Traits {
        app_name:&n.app_name, app_icon:&n.app_icon, category:&n.category,
        desktop_entry:&n.desktop_entry, summary:&n.summary, body:&n.body,
        urgency:n.urgency.criteria_label(),
        anchor:if n.surface_associated {
            n.surface_anchor.as_deref().or_else(||
                config.global.get("anchor").map(String::as_str)
            ).unwrap_or("top-right")
        }else{""},
        output:if n.surface_associated {
            n.surface_output_override.as_ref()
                .map(|value|value.as_deref().unwrap_or(""))
                .unwrap_or(observed_output.as_deref().unwrap_or(""))
        }else{""},
        surface_associated:n.surface_associated,
        output_known:n.surface_associated && n.surface_output_override
            .as_ref().map(Option::is_some).unwrap_or(observed_output.is_some()),
        modes:&mode_views,
        requested_timeout_ms:n.requested_timeout_ms,
        actionable:!n.actions.is_empty(),
        group_index,hidden:false,
    };
    config.style_for(&traits)
}

/// Project Mako-visible group members and enforce max-visible per group.
/// The compositor frame builder also renders an overflow placeholder for
/// hidden notifications; secondary members can be explicitly re-shown by style.
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
impl VisibleMember {
    /// Pinned Mako notification.c:group_notifications sets group_count
    /// even for ONE matching member when group-by is enabled. Without
    /// an actual grouping pass, a fresh C notification has count zero.
    /// The former >=2 shortcut erased the source's singleton count.
    pub fn formatted_group_count(&self)->usize{
        if self.notification.source_group_count_set||self.group_count>=2{
            self.group_count
        }else{
            0
        }
    }
}

pub fn visible_members(runtime:&SharedRuntime)
    -> io::Result<Vec<(Notification,usize)>>
{
    Ok(visible_members_indexed(runtime)?.into_iter()
        .map(|m|(m.notification,m.group_count)).collect())
}

/// The compositor renders each Mako surface independently. Keep its
/// projection counters independent of the legacy single-popup composite.
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct SurfaceVisibility {
    pub surface_key:(String,String,String),
    pub notification_ids:Vec<u32>,
    pub hidden_count:usize,
    pub total_count:usize,
}

#[derive(Debug)]
struct VisibleProjection {
    members:Vec<VisibleMember>,
    hidden_count:usize,
    total_count:usize,
    surfaces:Vec<SurfaceVisibility>,
}

/// Source-backed surface-local projection for the future multi-surface
/// presenter. Does not create or map any Wayland surface itself.
pub fn surface_visibility(runtime:&SharedRuntime)->io::Result<Vec<SurfaceVisibility>>{
    Ok(visible_projection(runtime)?.surfaces)
}

pub fn visible_members_indexed(runtime:&SharedRuntime)
    -> io::Result<Vec<VisibleMember>>
{
    Ok(visible_projection(runtime)?.members)
}

fn visible_projection(runtime:&SharedRuntime)->io::Result<VisibleProjection>{
    // Keep lock acquisition order consistent with SharedRuntime::schedule:
    // store first, modes second. The reverse order can deadlock with a
    // concurrent Notify() while the popup thread is reading groups.
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let by_id=store.active();
    let total_count=by_id.len();
    let mut selected=Vec::new();
    let mut hidden_count=0usize;
    let mut surface_views=Vec::<SurfaceVisibility>::new();
    // Surface objects persist independently of visible notifications. Use
    // active notification order to produce a deterministic view for each
    // associated identity (the actual presenter owns surface creation).
    for note in by_id{
        let key=note.surface_spec.clone()
            .unwrap_or_else(||("".into(),"top".into(),"top-right".into()));
        if let Some(view)=surface_views.iter_mut()
            .find(|v|v.surface_key==key){
            view.total_count+=1;
        }else{
            surface_views.push(SurfaceVisibility{
                surface_key:key,notification_ids:Vec::new(),
                hidden_count:0,total_count:1,
            });
        }
    }
    // Mako render.c considers only the notifications attached to its
    // current surface, so max-visible is independently counted per
    // (configured output, layer, anchor) identity.
    let mut displayed_groups=std::collections::HashMap::<(String,String,String),usize>::new();
    for group in group_ordered_scoped(runtime,by_id,&modes).map_err(other)? {
        let count=group.ids.len();
        for (index,id) in group.ids.iter().enumerate() {
            if let Some(note)=by_id.iter().find(|n|&n.id==id) {
                let (group_index,real_count)=if note.grouping_applied{
                    (note.group_index,note.group_count)
                }else{
                    (if count>1{index as i32}else{-1},count)
                };
                let style=style_for_note_index(runtime,note,&modes,group_index);
                let limit=style.get("max-visible")
                    .and_then(|v|crate::config::parse_signed_mako(v)).unwrap_or(5);
                let surface_key=note.surface_spec.clone()
                    .unwrap_or_else(||("".into(),"top".into(),"top-right".into()));
                let used=displayed_groups.get(&surface_key).copied().unwrap_or(0);
                if limit>=0&&used>=limit as usize {
                    // Upstream increments hidden count for every excluded
                    // notification, including secondary grouped members.
                    hidden_count+=1;
                    if let Some(view)=surface_views.iter_mut()
                        .find(|v|v.surface_key==surface_key){
                        view.hidden_count+=1;
                    }
                    continue;
                }
                if style.get("invisible").is_some_and(|v|
                    v=="1"||v.eq_ignore_ascii_case("true")) {
                    continue;
                }
                selected.push(VisibleMember{
                    notification:note.clone(),group_count:real_count,group_index,
                });
                if let Some(view)=surface_views.iter_mut()
                    .find(|v|v.surface_key==surface_key){
                    view.notification_ids.push(note.id);
                }
                if group_index<=0 {
                    *displayed_groups.entry(surface_key).or_default()+=1;
                }
            }
        }
    }
    Ok(VisibleProjection{
        members:selected,hidden_count,total_count,surfaces:surface_views,
    })
}

/// Count real groups eligible for display, excluding explicitly invisible
/// categories. This excludes them from synthetic hidden-count placeholders.
pub fn displayable_groups(runtime:&SharedRuntime)->io::Result<usize>{
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let mut count=0usize;
    for group in group_ordered_scoped(runtime,store.active(),&modes)
        .map_err(other)?{
        if let Some(id)=group.ids.first(){
            if let Some(n)=store.active().iter().find(|n|&n.id==id){
                let style=if n.grouping_applied{
                    style_for_note_index(runtime,n,&modes,n.group_index)
                }else{
                    style_for_note_group(runtime,n,&modes,group.ids.len())
                };
                if !style.get("invisible").is_some_and(|v|
                    v=="1"||v.eq_ignore_ascii_case("true")){
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
    snapshot_frame_scaled(runtime,1)
}

/// All child frames and their spacing use the compositor's physical pixel
/// scale; the corresponding layer-shell geometry is still logical.
pub fn snapshot_frame_scaled(runtime:&SharedRuntime,pixel_scale:u32)
    -> io::Result<Option<PixelFrame>>
{
    snapshot_frame_with_icon_scale(runtime,pixel_scale,pixel_scale)
}

/// Render at the chosen output scale while choosing the same themed icon
/// size as Mako, which considers the maximum scale of all attached monitors.
fn snapshot_frame_with_icon_scale(
    runtime:&SharedRuntime,pixel_scale:u32,icon_lookup_scale:u32,
)->io::Result<Option<PixelFrame>>{
    snapshot_frame_for_surface(
        runtime,pixel_scale,icon_lookup_scale,None,OutputSubpixel::Unknown
    )
}

/// Rasterize ONLY the notifications and overflow placeholder belonging to
/// one Mako (configured output, layer, anchor) surface identity. This is a
/// headless/isolated rendering primitive; it does not map a Wayland surface.
pub fn snapshot_surface_frame_scaled(
    runtime:&SharedRuntime,surface_key:&(String,String,String),pixel_scale:u32,
)->io::Result<Option<PixelFrame>>{
    snapshot_frame_for_surface(
        runtime,pixel_scale,pixel_scale,Some(surface_key),OutputSubpixel::Unknown
    )
}

fn snapshot_frame_for_surface(
    runtime:&SharedRuntime,pixel_scale:u32,icon_lookup_scale:u32,
    surface_key:Option<&(String,String,String)>,subpixel:OutputSubpixel,
)->io::Result<Option<PixelFrame>>{
    // The Wayland compositor may advertise scales above 8. Preserve the
    // reported positive i32 value; downstream frame measurements enforce
    // the actual SHM bounds instead of an arbitrary compositor scale cap.
    if pixel_scale==0 || pixel_scale>i32::MAX as u32 {
        return Err(other("invalid compositor pixel scale"));
    }
    let scale=pixel_scale as i32;
    let scale_margin=|raw:&String| {
        PopupMargins::parse_mako(raw).map(|m|PopupMargins{
            top:m.top.saturating_mul(scale),right:m.right.saturating_mul(scale),
            bottom:m.bottom.saturating_mul(scale),left:m.left.saturating_mul(scale),
        })
    };
    let config=runtime.config();
    let projection=visible_projection(runtime)?;
    let (notes,total,hidden)=if let Some(key)=surface_key{
        let Some(view)=projection.surfaces.iter()
            .find(|view|&view.surface_key==key)
        else{return Ok(None)};
        let notes=projection.members.into_iter()
            .filter(|member|member.notification.surface_spec.as_ref()==Some(key))
            .collect::<Vec<_>>();
        (notes,view.total_count,view.hidden_count)
    }else{
        (projection.members,projection.total_count,projection.hidden_count)
    };
    // A mapped surface shares one compositor-entered output among its
    // notifications. An explicitly unmapped surface must not borrow another
    // surface's observed monitor. Use the legacy presenter observation only
    // when no independent surface event exists.
    let surface_output=if let Some(key)=surface_key{
        let store=runtime.store.lock().map_err(|_|poisoned())?;
        let observed=store.active().iter()
            .find(|note|note.surface_spec.as_ref()==Some(key))
            .and_then(|note|note.surface_output_override.clone());
        Some(observed.unwrap_or_else(||runtime.surface_output()))
    }else{
        // The single-surface presenter also receives wl_surface.enter.
        // Pinned render.c:443-446 binds the synthetic hidden notif to
        // that surface before matching [hidden output=NAME]. Dropping the
        // observed name here wrongly suppresses valid surface-scoped rules.
        runtime.surface_output().map(Some)
    };
    let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
    let mut frames=Vec::with_capacity(notes.len());
    let mut margins=Vec::with_capacity(notes.len());
    for entry in &notes {
        let note=&entry.notification;
        let style=style_for_note_index(runtime,note,&modes,entry.group_index);
        margins.push(style.get("margin")
            .and_then(scale_margin)
            .unwrap_or(PopupMargins::uniform(10*scale)));
        frames.push(render_notification_group_with_output_subpixel(
            note,&style,entry.formatted_group_count(),
            pixel_scale,icon_lookup_scale,subpixel
        ).map_err(other)?);
    }
    let visible_count=frames.len();
    if hidden>0 {
        // Mako v1.11 render.c calls create_notification() before applying
        // [hidden] styling, including when the pseudo-notification will
        // itself be invisible. Each rendered surface consumes one ID
        // per overflow redraw, without adding a sender-visible item.
        runtime.store.lock().map_err(|_|poisoned())?
            .reserve_hidden_render_id();
        let mode_views:Vec<&str>=modes.iter().map(String::as_str).collect();
        let traits=Traits{
            app_name:"",app_icon:"",category:"",desktop_entry:"",
            summary:"",body:"",urgency:"normal",
            anchor:surface_key.map(|key|key.2.as_str()).unwrap_or_else(|| 
                config.global.get("anchor").map(String::as_str).unwrap_or("top-right")),
            output:surface_output.as_ref()
                .map(|output|output.as_deref().unwrap_or(""))
                .unwrap_or_else(||config.global.get("output")
                    .map(String::as_str).unwrap_or("")),
            surface_associated:true,
            output_known:surface_output.as_ref()
                .is_some_and(Option::is_some),
            modes:&mode_views,requested_timeout_ms:-1,
            actionable:false,group_index:-1,hidden:true,
        };
        let mut style=config.style_for(&traits);
        if !style.get("invisible").is_some_and(|x|x=="1"||x=="true"){
            // Pinned Mako v1.11 render.c:113-115 clamps EACH notification
            // to the layer surface's compositor-confirmed width BEFORE
            // Pango layout, including this synthetic [hidden] note.
            // render.c:424-430 excludes the synthetic note from the
            // requested surface width. wayland.c:361-379 then supplies the
            // configured width; Mako's next frame uses that bound.
            // A wider hidden rule must wrap at that surface width instead
            // of measuring unconstrained lines and clipping afterward.
            if visible_count>0 {
                let dimensions=frames.iter().map(|frame|(frame.width,frame.height))
                    .collect::<Vec<_>>();
                let anchor=surface_key.map(|key|key.2.as_str()).unwrap_or_else(||
                    config.global.get("anchor").map(String::as_str).unwrap_or("top-right"));
                let (canvas_width,_,_)=mako_stack_geometry(
                    &dimensions,&margins,stack_alignment(anchor)
                ).map_err(other)?;
                let logical_width=(canvas_width/pixel_scale).max(1);
                let configured=style.get("width")
                    .and_then(|v|crate::config::parse_nonnegative_mako(v))
                    .unwrap_or(300);
                if configured>logical_width {
                    style.insert("width".into(),logical_width.to_string());
                }
            }
            frames.push(render_hidden_placeholder_with_subpixel(
                hidden,total,&style,pixel_scale,subpixel
            ).map_err(other)?);
            margins.push(style.get("margin")
                .and_then(scale_margin)
                .unwrap_or(PopupMargins::uniform(10*scale)));
        }
    }
    let anchor=surface_key.map(|key|key.2.as_str()).unwrap_or_else(||
        config.global.get("anchor").map(String::as_str).unwrap_or("top-right"));
    if visible_count<frames.len(){
        compose_mako_stack_with_visible_count(
            &frames,&margins,stack_alignment(anchor),visible_count
        ).map_err(other)
    }else{
        compose_mako_stack(&frames,&margins,stack_alignment(anchor)).map_err(other)
    }
}

struct DesktopClick {
    // Preserve the clicked notification and binding across the potentially
    // asynchronous compositor XDG activation-token response. A second
    // hit-test after that wait can otherwise target a newer popup.
    target:Notification,
    instance:u64,
    binding:crate::hooks::NotifyBinding,
    activation_token:Option<String>,
}

/// A replaced or removed notification is no longer the one whose hotspot
/// produced the compositor event. Never retarget the same click to a
/// different notification or a newer payload with the same Freedesktop ID.
fn click_target_is_current(
    runtime:&SharedRuntime,target:&Notification,instance:u64
)->io::Result<bool>{
    let store=runtime.store.lock().map_err(|_|poisoned())?;
    Ok(store.active_instance(target.id)==Some(instance)
       && store.active().iter().any(|note|note==target))
}

fn render_loop(runtime:SharedRuntime,sender:tokio::sync::mpsc::UnboundedSender<DesktopClick>) -> io::Result<()> {
    let mut viewer=LivePresenter::connect().map_err(other)?;
    let (mut previous, mut previous_modes)=(Vec::new(),Vec::new());
    let mut previous_revision=u64::MAX;
    let mut previous_scale=0u32;
    let mut previous_icon_scale=0u32;
    let mut previous_subpixel=OutputSubpixel::Unknown;
    let mut expected_surface=false;
    loop {
        let notes: Vec<_>=runtime.store.lock().map_err(|_|poisoned())?
            .active().iter().cloned().collect();
        let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
        // Read Wayland output scale events before deciding whether a fresh
        // logical image must be committed with new physical SHM dimensions.
        viewer.poll_events().map_err(other)?;
        // Mako matches [output=...] to the compositor-assigned surface,
        // including when the layer shell was given no explicit output.
        // A surface recreation temporarily loses its enter event: keep the
        // previous known monitor until the replacement is mapped.
        let assigned_output=viewer.current_output_name().map(str::to_owned);
        if (assigned_output.is_some() || !viewer.has_active_surface()) &&
           runtime.set_surface_output(assigned_output.as_deref()).map_err(other)? {
            runtime.reapply_active_grouping().map_err(other)?;
        }
        let config=runtime.config();
        let selected_output=config.global.get("output")
            .map(String::as_str).unwrap_or("");
        // A configured monitor may be unplugged. Do not terminate the D-Bus
        // service or keep a stale layer surface; retry after registry updates.
        if !viewer.has_output(selected_output) {
            viewer.clear();
            std::thread::sleep(Duration::from_millis(45));
            continue;
        }
        let scale=viewer.output_scale(selected_output);
        let icon_scale=viewer.maximum_output_scale();
        let subpixel=viewer.output_subpixel();
        runtime.set_icon_creation_scale(icon_scale);
        let revision=runtime.revision();
        if notes!=previous || modes!=previous_modes ||
            revision!=previous_revision || scale!=previous_scale ||
            icon_scale!=previous_icon_scale ||
            subpixel!=previous_subpixel ||
            (expected_surface && !viewer.has_active_surface()) {

            let frame=snapshot_frame_for_surface(
                &runtime,scale,icon_scale,None,subpixel
            )?;
            let corner=config.global.get("anchor")
                .and_then(|s|ScreenCorner::from_mako(s))
                .unwrap_or(ScreenCorner::TopRight);
            let margins=outer_margins(&config);
            let layer=config.global.get("layer")
                .and_then(|s|PopupLayer::from_mako(s))
                .unwrap_or(PopupLayer::Top);
            let output=config.global.get("output").map(String::as_str).unwrap_or("");
            if frame.is_none()||!viewer.buffer_backpressure(){
                let input_rects=if frame.is_some(){
                    crate::input::input_rectangles_at_output_scale(
                        &runtime,None,icon_scale,scale,subpixel
                    )?
                }else{Vec::new()};
                viewer.update_native_frame_with_input_rectangles(
                    frame.as_ref(),corner,margins,layer,output,&input_rects
                ).map_err(other)?;
                previous=notes;
                previous_modes=modes;
                previous_revision=revision;
                previous_scale=scale;
                previous_icon_scale=icon_scale;
                previous_subpixel=subpixel;
                expected_surface=frame.is_some();
            }
            // Otherwise leave the previous snapshots unchanged: the next
            // compositor buffer release retries the newest frame, without
            // dropping notification or input events.
        }
        for click in viewer.take_clicks() {
            // Snapshot only small (ID, instance) pairs before hit-testing.
            // Notifications may carry multi-megabyte inline icons; cloning
            // every payload once more per click would waste memory and time.
            let instances={
                let store=runtime.store.lock().map_err(|_|poisoned())?;
                store.active().iter().filter_map(|note|
                    store.active_instance(note.id).map(|instance|
                        (note.id,instance))
                ).collect::<Vec<_>>()
            };
            let Some((id,binding))=crate::input::hit_test_at_output_scale(
                &runtime,click,icon_scale,scale,subpixel
            ).map_err(other)? else{continue};
            let Some((_,instance))=instances.into_iter()
                .find(|(candidate,_)|*candidate==id) else{continue};
            let target={
                let store=runtime.store.lock().map_err(|_|poisoned())?;
                if store.active_instance(id)!=Some(instance){continue;}
                store.active().iter().find(|note|note.id==id).cloned()
            };
            let Some(target)=target else{continue};
            if !click_target_is_current(&runtime,&target,instance)? {continue;}
            // Request activation only for an actually matching sender action.
            // Dismissals and missing default keys must not incur a token wait.
            let invokes_action=crate::input::should_request_activation_token(
                &runtime,id,&binding).map_err(other)?;
            let activation_token=if invokes_action {
                viewer.request_activation_token(click.serial).map_err(other)?
            }else{None};
            if sender.send(DesktopClick{target,instance,binding,activation_token}).is_err() {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(45));
    }
}

/// Experimental multi-surface implementation. Every Mako surface identity
/// owns a separate Wayland connection, compositor output observation, buffer
/// lifecycle and click coordinate space. This avoids incorrectly merging
/// independently anchored/output-targeted notifications into one popup.
/// It is private-bus-only and deliberately separate from the established
/// one-surface presenter until live Hyprland acceptance is available.
fn render_loop_multi(
    runtime:SharedRuntime,
    sender:tokio::sync::mpsc::UnboundedSender<DesktopClick>,
)->io::Result<()>{
    type Key=(String,String,String);
    type FrameStamp=(Vec<Notification>,Vec<String>,u64,u32,u32,OutputSubpixel,bool);
    let mut viewers=std::collections::HashMap::<Key,LivePresenter>::new();
    let mut rendered=std::collections::HashMap::<Key,FrameStamp>::new();
    // Mako tracks wl_output scale independently of notification surfaces.
    // Keep a registry-only observer even with zero active notifications:
    // otherwise changing DPI while the popup list is empty leaves the
    // NEXT Notify stuck with the previous icon-search scale.
    let mut output_observer=LivePresenter::connect().map_err(other)?;
    loop{
        output_observer.poll_events().map_err(other)?;
        runtime.set_icon_creation_scale(output_observer.maximum_output_scale());
        let surfaces=surface_visibility(&runtime)?;
        let keys=surfaces.iter().map(|s|s.surface_key.clone())
            .collect::<std::collections::HashSet<_>>();
        // Dropping a presenter destroys only that surface's layer-shell
        // objects; Mako's notification store/history is unaffected.
        viewers.retain(|key,_|keys.contains(key));
        rendered.retain(|key,_|keys.contains(key));
        let all_notes=runtime.store.lock().map_err(|_|poisoned())?
            .active().iter().cloned().collect::<Vec<_>>();
        let modes=runtime.modes().lock().map_err(|_|poisoned())?.clone();
        let config=runtime.config();
        for view in surfaces{
            let key=view.surface_key;
            if !viewers.contains_key(&key){
                viewers.insert(key.clone(),LivePresenter::connect().map_err(other)?);
            }
            let viewer=viewers.get_mut(&key).ok_or_else(poisoned)?;
            viewer.poll_events().map_err(other)?;
            let observed=viewer.current_output_name().map(str::to_owned);
            if observed.is_some()||!viewer.has_active_surface(){
                if let Some(id)=all_notes.iter()
                    .find(|note|note.surface_spec.as_ref()==Some(&key))
                    .map(|note|note.id){
                    runtime.set_notification_surface_output(id,observed.as_deref())
                        .map_err(other)?;
                }
            }
            // Output= is the layer-shell placement request, not an
            // [output] criterion. Only wl_surface.enter supplies the latter.
            if !viewer.has_output(&key.0){
                viewer.clear();
                rendered.remove(&key);
                continue;
            }
            let scale=viewer.output_scale(&key.0);
            let icon_scale=viewer.maximum_output_scale();
            let subpixel=viewer.output_subpixel();
            runtime.set_icon_creation_scale(icon_scale);
            let revision=runtime.revision();
            let current_notes=all_notes.iter()
                .filter(|note|note.surface_spec.as_ref()==Some(&key))
                .cloned().collect::<Vec<_>>();
            let changed=rendered.get(&key).is_none_or(|stamp|{
                stamp.0!=current_notes||stamp.1!=modes||stamp.2!=revision||
                stamp.3!=scale||stamp.4!=icon_scale||stamp.5!=subpixel||
                (stamp.6&&!viewer.has_active_surface())
            });
            if changed{
                let frame=snapshot_frame_for_surface(
                    &runtime,scale,icon_scale,Some(&key),subpixel
                )?;
                let corner=ScreenCorner::from_mako(&key.2)
                    .ok_or_else(||other("invalid surface anchor"))?;
                let layer=PopupLayer::from_mako(&key.1)
                    .ok_or_else(||other("invalid surface layer"))?;
                if frame.is_none()||!viewer.buffer_backpressure(){
                    let input_rects=if frame.is_some(){
                        crate::input::input_rectangles_at_output_scale(
                            &runtime,Some(&key),icon_scale,scale,subpixel
                        )?
                    }else{Vec::new()};
                    viewer.update_native_frame_with_input_rectangles(
                        frame.as_ref(),corner,outer_margins(&config),layer,&key.0,
                        &input_rects,
                    ).map_err(other)?;
                    rendered.insert(key.clone(),(
                        current_notes,modes.clone(),revision,scale,icon_scale,
                        subpixel,frame.is_some(),
                    ));
                }
            }
            for click in viewer.take_clicks(){
                let Some((id,binding))=crate::input::hit_test_on_surface_at_output_scale(
                    &runtime,click,&key,icon_scale,scale,subpixel
                ).map_err(other)? else{continue};
                let (target,instance)={
                    let store=runtime.store.lock().map_err(|_|poisoned())?;
                    let instance=store.active_instance(id);
                    let note=store.active().iter().find(|n|n.id==id).cloned();
                    (note,instance)
                };
                let (Some(target),Some(instance))=(target,instance) else{continue};
                if !click_target_is_current(&runtime,&target,instance)?{continue;}
                let needs_token=crate::input::should_request_activation_token(
                    &runtime,id,&binding
                ).map_err(other)?;
                let activation_token=if needs_token{
                    viewer.request_activation_token(click.serial).map_err(other)?
                }else{None};
                if sender.send(DesktopClick{
                    target,instance,binding,activation_token,
                }).is_err(){return Ok(());}
            }
        }
        std::thread::sleep(Duration::from_millis(45));
    }
}

/// Drive the D-Bus service and the live Wayland preview in one process, each
/// with its own event-loop thread. Both components share notification state.
fn initialize_icon_creation_scale(runtime:&SharedRuntime)->io::Result<()>{
    // Mako owns wl_output scale information before it accepts Notify.
    // Preparing a temporary registry-only presenter avoids a race where
    // the first private D-Bus message arrives before the render loop starts.
    // No layer surface is created and the real session is never touched.
    let mut presenter=LivePresenter::connect().map_err(other)?;
    // Two compositor event barriers let wl_output scale notifications
    // settle even when the output layout changed just before process start.
    // Do not accept Notify while the initial monitor-scale snapshot is stale.
    presenter.poll_events().map_err(other)?;
    let scale=presenter.maximum_output_scale();
    eprintln!("Posthorn initial compositor maximum icon scale: {scale}");
    runtime.set_icon_creation_scale(scale);
    Ok(())
}

pub async fn run_private_desktop(config: crate::config::Config)
    -> Result<(),Box<dyn std::error::Error + Send + Sync>>
{
    let runtime=SharedRuntime::new(config);
    initialize_icon_creation_scale(&runtime)?;
    let (conn,runtime)=crate::service::open_private_runtime(runtime).await?;
    run_private_desktop_instance(conn,runtime).await
}

pub async fn run_private_desktop_from_path(path:&std::path::Path)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    run_private_desktop_from_path_with_cli_overrides(path,&[]).await
}

/// Private Wayland parity path: apply pinned Mako's CLI overrides *after*
/// the source config, and preserve them for live Reload. This entrypoint
/// still owns only org.posthorn.Development, never the production bus.
pub async fn run_private_desktop_from_path_with_cli_overrides(
    path:&std::path::Path,overrides:&[(String,String)],
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let runtime=SharedRuntime::from_path_with_cli_overrides(path,overrides)?;
    initialize_icon_creation_scale(&runtime)?;
    let (conn,runtime)=crate::service::open_private_runtime(runtime).await?;
    run_private_desktop_instance(conn,runtime).await
}

/// No path supplied: discover the same isolated HOME/XDG config as Mako.
pub async fn run_private_desktop_from_defaults_with_cli_overrides(
    overrides:&[(String,String)],
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let runtime=SharedRuntime::from_mako_defaults_with_cli_overrides(overrides)?;
    initialize_icon_creation_scale(&runtime)?;
    let (conn,runtime)=crate::service::open_private_runtime(runtime).await?;
    run_private_desktop_instance(conn,runtime).await
}

/// Private-bus-only multi-surface acceptance path. Does not register
/// org.freedesktop.Notifications or replace the installed Mako daemon.
pub async fn run_private_multi_desktop_from_path(path:&std::path::Path)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    run_private_multi_desktop_from_path_with_cli_overrides(path,&[]).await
}
pub async fn run_private_multi_desktop_from_path_with_cli_overrides(
    path:&std::path::Path,overrides:&[(String,String)],
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let runtime=SharedRuntime::from_path_with_cli_overrides(path,overrides)?;
    initialize_icon_creation_scale(&runtime)?;
    let (conn,runtime)=crate::service::open_private_runtime(runtime).await?;
    run_private_desktop_instance_with_mode(conn,runtime,true).await
}

pub async fn run_private_multi_desktop_from_defaults_with_cli_overrides(
    overrides:&[(String,String)],
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let runtime=SharedRuntime::from_mako_defaults_with_cli_overrides(overrides)?;
    initialize_icon_creation_scale(&runtime)?;
    let (conn,runtime)=crate::service::open_private_runtime(runtime).await?;
    run_private_desktop_instance_with_mode(conn,runtime,true).await
}

async fn run_private_desktop_instance(conn:zbus::Connection,runtime:SharedRuntime)
    -> Result<(),Box<dyn std::error::Error+Send+Sync>>
{
    run_private_desktop_instance_with_mode(conn,runtime,false).await
}

async fn run_private_desktop_instance_with_mode(
    conn:zbus::Connection,runtime:SharedRuntime,multiple_surfaces:bool,
)->Result<(),Box<dyn std::error::Error+Send+Sync>>{
    let visual_runtime=runtime.clone();
    let (sender,mut clicks)=tokio::sync::mpsc::unbounded_channel();
    let mut visual=tokio::task::spawn_blocking(move || {
        if multiple_surfaces{
            render_loop_multi(visual_runtime,sender)
        }else{
            render_loop(visual_runtime,sender)
        }
    });
    let mut timer=tokio::time::interval(Duration::from_millis(25));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    eprintln!(
        "Posthorn private desktop preview: bus org.posthorn.Development only; Mako remains active"
    );
    loop {
        tokio::select! {
            click=clicks.recv() => {
                if let Some(event)=click {
                    if click_target_is_current(&runtime,&event.target,event.instance)? {
                        crate::input::dispatch_binding(
                            &conn,&runtime,event.target.id,event.binding,
                            event.activation_token.as_deref()
                        ).await?;
                    }
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

    #[test]
    fn compositor_click_target_remains_bound_to_original_notification() {
        let runtime=SharedRuntime::new(Config::parse("").unwrap());
        let old=runtime.store.lock().unwrap().push(Request{
            summary:"Original clicked popup".into(),..Request::default()
        });
        let snapshot=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==old).unwrap().clone();
        let original_instance=runtime.store.lock().unwrap()
            .active_instance(old).unwrap();
        assert!(click_target_is_current(&runtime,&snapshot,original_instance).unwrap());
        let newer=runtime.store.lock().unwrap().push(Request{
            summary:"New popup covering original hotspot".into(),..Request::default()
        });
        // A new popup does not retroactively retarget an already captured
        // click; the original notification still exists unchanged.
        assert!(click_target_is_current(&runtime,&snapshot,original_instance).unwrap());
        assert_ne!(old,newer);
        // Even byte-for-byte identical replacements renew the instance.
        // Notification value equality alone cannot defend this race.
        assert_eq!(runtime.store.lock().unwrap().push(Request{
            replaces_id:old,summary:"Original clicked popup".into(),
            ..Request::default()
        }),old);
        let identical=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==old).unwrap().clone();
        assert_eq!(identical,snapshot);
        assert!(!click_target_is_current(&runtime,&snapshot,original_instance).unwrap(),
            "an identical Notify replacement must invalidate the old click");
        // Freedesktop replaces_id keeps the numeric ID. Identity must still
        // distinguish the new payload from the clicked one.
        assert_eq!(runtime.store.lock().unwrap().push(Request{
            replaces_id:old,summary:"Replaced popup".into(),..Request::default()
        }),old);
        assert!(!click_target_is_current(&runtime,&snapshot,original_instance).unwrap());
        let replacement=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==old).unwrap().clone();
        let replacement_instance=runtime.store.lock().unwrap()
            .active_instance(old).unwrap();
        assert_ne!(original_instance,replacement_instance);
        assert!(click_target_is_current(&runtime,&replacement,replacement_instance).unwrap());
        runtime.close(old,CloseReason::Dismissed,false).unwrap();
        assert!(!click_target_is_current(&runtime,&replacement,replacement_instance).unwrap());
    }

    #[test]
    fn all_hidden_notifications_do_not_create_synthetic_only_canvas(){
        // Pinned Mako render.c:360-361,424-430,472-484 only increases
        // max_width for real visible notifications. With max-visible=0,
        // the synthetic hidden-count popup contributes height, but no
        // drawable layer-shell width. Creating a full-width placeholder
        // in that case would be a Posthorn-only visual regression.
        let runtime=SharedRuntime::new(Config::parse(
            "width=96\nheight=60\nmargin=3\nmax-visible=0\n\
             [hidden]\nbackground-color=#E5923C\n"
        ).unwrap());
        let first=runtime.store.lock().unwrap().push(Request::default());
        let second=runtime.store.lock().unwrap().push(Request::default());
        assert_eq!((first,second),(1,2));
        assert!(snapshot_frame(&runtime).unwrap().is_none(),
            "hidden-count alone must not invent a nonzero Mako surface width");
        assert!(snapshot_frame_scaled(&runtime,2).unwrap().is_none(),
            "HiDPI cannot turn zero real-visible width into a popup");
        assert_eq!(runtime.store.lock().unwrap().active().len(),2,
            "synthetic hidden redraw cannot remove real D-Bus entries");
        let third=runtime.store.lock().unwrap().push(Request::default());
        assert_eq!(third,5,
            "two zero-visible redraws still allocate synthetic IDs 3 and 4");
    }

    #[test]
    fn wider_hidden_style_is_clamped_to_configured_surface_before_pango(){
        // Source render.c:113-115 caps notif_width to surface->width
        // BEFORE Pango sizing, even for [hidden] at render.c:472-474.
        // wayland.c:361-379 confirms surface width from the compositor.
        // The wider-style bitmap must be the negative control, not the
        // acceptance reference, when visible notifications are narrower.
        let format="Overflow status: %h hidden among %t total notifications, with deliberately long text";
        let config=Config::parse(&format!(
            "width=84\nheight=200\nborder-size=0\npadding=0\nmargin=0\n\
             font=Sans 20\nicons=0\nmax-visible=1\nformat=%s\n\
             [hidden]\nwidth=260\nformat={format}\nbackground-color=#9371B2\n",
        )).unwrap();
        let runtime=SharedRuntime::new(config);
        let first=runtime.store.lock().unwrap().push(Request{
            summary:"Older hidden".into(),..Request::default()
        });
        let second=runtime.store.lock().unwrap().push(Request{
            summary:"Newer visible".into(),..Request::default()
        });
        let projection=visible_projection(&runtime).unwrap();
        assert_eq!((projection.hidden_count,projection.members.len()),(1,1));
        assert_eq!(projection.members[0].notification.id,second);
        assert_ne!(first,second);
        let entry=&projection.members[0];
        let modes=["default".to_owned()];
        let style=style_for_note_index(
            &runtime,&entry.notification,&modes,entry.group_index
        );
        let visible=render_notification_group_with_icon_scale(
            &entry.notification,&style,0,1,1
        ).unwrap();
        let mut hidden_style=runtime.config().global.clone();
        hidden_style.insert("width".into(),"260".into());
        hidden_style.insert("format".into(),format.into());
        hidden_style.insert("background-color".into(),"#9371B2".into());
        let wide=render_hidden_placeholder_scaled(1,2,&hidden_style,1).unwrap();
        let mut surface_clamped=hidden_style.clone();
        surface_clamped.insert("width".into(),"84".into());
        let narrow=render_hidden_placeholder_scaled(
            1,2,&surface_clamped,1
        ).unwrap();
        assert!(narrow.height>wide.height,
            "negative control: the configured surface width causes extra wrapping");
        let no_margin=PopupMargins::uniform(0);
        let reference=compose_mako_stack_with_visible_count(
            &[visible.clone(),narrow],&[no_margin,no_margin],StackAlignment::Right,1
        ).unwrap().unwrap();
        let incorrectly_wide=compose_mako_stack_with_visible_count(
            &[visible,wide],&[no_margin,no_margin],StackAlignment::Right,1
        ).unwrap().unwrap();
        let actual=snapshot_frame(&runtime).unwrap().unwrap();
        assert_eq!(actual.width,84,
            "synthetic overflow cannot widen the real notification canvas");
        assert_eq!(actual.height,reference.height,
            "hidden Pango height must reflect the configured layer surface width");
        assert_eq!(actual.pixels,reference.pixels,
            "hidden bitmap must be laid out at surface width before painting");
        assert_ne!(actual.height,incorrectly_wide.height,
            "negative control must reject unclamped [hidden] height");
    }

    #[test]
    fn source_group_singleton_format_differs_from_truly_ungrouped(){
        let mut store=crate::notification::NotificationStore::new(4);
        store.push(crate::notification::Request::default());
        let note=store.active()[0].clone();
        let mk=|applied:bool,count:usize|{
            let mut n=note.clone();
            n.grouping_applied=true;
            n.source_group_count_set=applied;
            VisibleMember{
                notification:n,group_count:count,
                group_index:if applied{0}else{-1},
            }
        };
        assert_eq!(mk(false,1).formatted_group_count(),0);
        assert_eq!(mk(true,1).formatted_group_count(),1,
            "pinned Mako group_notifications(count=1) must retain %g=1");
        assert_eq!(mk(true,2).formatted_group_count(),2);
    }

    #[test]
    fn hidden_placeholder_redraw_consumes_mako_id_but_not_active_slot() {
        let runtime=SharedRuntime::new(Config::parse(
            "width=90\nheight=30\nmax-visible=1\n"
        ).unwrap());
        let a=runtime.store.lock().unwrap().push(Request{
            summary:"first".into(),..Request::default()
        });
        let b=runtime.store.lock().unwrap().push(Request{
            summary:"second".into(),..Request::default()
        });
        assert_eq!((a,b),(1,2));
        assert_eq!(runtime.store.lock().unwrap().active().len(),2);
        assert!(snapshot_frame(&runtime).unwrap().is_some());
        assert_eq!(runtime.store.lock().unwrap().active().len(),2,
            "overflow pseudo-notification must not reach ListNotifications");
        // Source create_notification allocates a fresh ID for each redraw.
        assert!(snapshot_frame(&runtime).unwrap().is_some());
        let c=runtime.store.lock().unwrap().push(Request::default());
        assert_eq!(c,5,"two overflow redraws must consume IDs 3 and 4");
        // With max-visible still one, another redraw consumes ID 6.
        assert!(snapshot_frame(&runtime).unwrap().is_some());
        runtime.close(b,CloseReason::Dismissed,false).unwrap();
        runtime.close(c,CloseReason::Dismissed,false).unwrap();
        // Only one notification remains: no placeholder, no synthetic ID.
        assert!(snapshot_frame(&runtime).unwrap().is_some());
        assert_eq!(runtime.store.lock().unwrap().push(Request::default()),7);
    }

    #[test]
    fn hidden_overflow_on_single_surface_matches_known_output_selector(){
        // render.c:443-446 assigns the already-mapped surface BEFORE
        // apply_each_criteria. The hidden pseudonote must see the same
        // wl_surface.enter output name as ordinary notifications.
        let runtime=SharedRuntime::new(Config::parse(
            "width=96\nheight=65\nmargin=0\npadding=0\nborder-size=0\nmax-visible=1\nformat=%s\n             [hidden]\nbackground-color=#224466FF\n             [hidden output=DP-1]\nbackground-color=#22CC11FF\n"
        ).unwrap());
        // Only the legacy single-surface path is at issue. Private
        // multi-surface rendering already forwards per-surface output.
        runtime.set_surface_output(Some("DP-1")).unwrap();
        runtime.store.lock().unwrap().push(Request::default());
        runtime.store.lock().unwrap().push(Request::default());
        let frame=snapshot_frame(&runtime).unwrap().unwrap();
        assert!(frame.pixels.chunks_exact(4)
            .any(|pixel|pixel==[0x11,0xCC,0x22,0xFF]),
            "hidden output-scoped styling lost compositor-entered output");
        assert!(!frame.pixels.chunks_exact(4)
            .any(|pixel|pixel==[0x66,0x44,0x22,0xFF]),
            "fallback hidden color was used instead of [hidden output=DP-1]");

        // When there has been no wl_surface.enter, output selectors do
        // not match, even if the configured output name is nonempty.
        runtime.set_surface_output(None).unwrap();
        let unmapped=snapshot_frame(&runtime).unwrap().unwrap();
        assert!(unmapped.pixels.chunks_exact(4)
            .any(|pixel|pixel==[0x66,0x44,0x22,0xFF]),
            "unknown output incorrectly matched the output-specific rule");
    }

    #[test]
    fn invisible_overflow_placeholder_still_consumes_mako_id() {
        let runtime=SharedRuntime::new(Config::parse(
            "width=90\nheight=30\nmax-visible=1\n[hidden]\ninvisible=1\n"
        ).unwrap());
        let first=runtime.store.lock().unwrap().push(Request::default());
        let second=runtime.store.lock().unwrap().push(Request::default());
        assert_eq!((first,second),(1,2));
        assert!(snapshot_frame(&runtime).unwrap().is_some(),
            "ordinary notification is still visible");
        assert_eq!(runtime.store.lock().unwrap().active().len(),2);
        assert_eq!(runtime.store.lock().unwrap().push(Request::default()),4,
            "Mako creates a hidden notification BEFORE checking invisible");
    }

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
        assert_eq!(after.height,measured+4,
            "pinned Mako v1.11.0 retains the top margin but omits the final bottom margin");
    }

    #[test]
    fn style_matching_uses_the_configured_anchor_instead_of_top_right() {
        let runtime=SharedRuntime::new(Config::parse(
            "anchor=bottom-left\n             [anchor=bottom-left]\nbackground-color=#715536\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let mode=vec!["default".to_string()];
        let fresh=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        assert_ne!(style_for_note(&runtime,&fresh,&mode)
            .get("background-color").map(String::as_str),Some("#715536"),
            "Mako cannot match anchor criteria before surface association");
        runtime.schedule_without_hook(id,std::time::Instant::now()).unwrap();
        let associated=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        let style=style_for_note(&runtime,&associated,&mode);
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
        let before=style_for_note(&runtime,&note,&modes);
        assert_ne!(before.get("background-color").map(String::as_str),
            Some("#305747"),"configured output is not a compositor enter event");
        runtime.schedule_without_hook(id,std::time::Instant::now()).unwrap();
        let note=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        runtime.set_surface_output(Some("DP-2")).unwrap();
        let style=style_for_note(&runtime,&note,&modes);
        assert_eq!(style.get("background-color").map(String::as_str),
            Some("#305747"));
    }

    #[test]
    fn compositor_assigned_output_overrides_configured_output_and_restyles() {
        let runtime=SharedRuntime::new(Config::parse(
            "output=DP-2\n[output=DP-2]\nbackground-color=#305747\n\
             [output=HEADLESS-1]\nbackground-color=#715536\n"
        ).unwrap());
        let id=runtime.store.lock().unwrap().push(Request::default());
        let note=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        let modes=["default".to_string()];
        let before=style_for_note(&runtime,&note,&modes);
        assert_ne!(before.get("background-color").map(String::as_str),
            Some("#305747"));
        runtime.schedule_without_hook(id,std::time::Instant::now()).unwrap();
        let note=runtime.store.lock().unwrap().active()
            .iter().find(|n|n.id==id).unwrap().clone();
        let revision=runtime.revision();
        assert!(runtime.set_surface_output(Some("HEADLESS-1")).unwrap());
        assert_eq!(runtime.revision(),revision+1);
        assert_eq!(style_for_note(&runtime,&note,&modes)
            .get("background-color").map(String::as_str),Some("#715536"));
        assert!(!runtime.set_surface_output(Some("HEADLESS-1")).unwrap());
        assert_eq!(runtime.revision(),revision+1);
        assert!(runtime.set_surface_output(Some("DP-2")).unwrap());
        assert_eq!(style_for_note(&runtime,&note,&modes)
            .get("background-color").map(String::as_str),Some("#305747"));
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
    fn superset_outer_margin_includes_all_criteria_not_just_global_style(){
        // config.c:apply_superset_style collects the largest value for each
        // directional field across every configured style. wayland.c then
        // uses this superstyle, not the notification's matching style.
        let config=Config::parse(
            "outer-margin=2,3,4,5\n\
             [category=urgent]\nouter-margin=18,-2,1,7\n\
             [app-name=other]\nouter-margin=-9,14,12,1\n"
        ).unwrap();
        assert_eq!(outer_margins(&config),PopupMargins{
            top:18,right:14,bottom:12,left:7,
        });
        // The second section need not match the currently displayed note:
        // upstream uses this single superset for EVERY layer surface.
        let config=Config::parse(
            "outer-margin=-5\n[category=unused]\nouter-margin=-3,-1,-8,-2\n"
        ).unwrap();
        assert_eq!(outer_margins(&config),PopupMargins::uniform(0),
            "the fresh superstyle starts at signed zero on each edge");
        let config=Config::parse(
            "margin=70\n[category=urgent]\nmargin=99\n"
        ).unwrap();
        assert_eq!(outer_margins(&config),PopupMargins::uniform(0),
            "inner notification margins never change layer-shell outer margins");
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
    fn surface_specific_frames_do_not_mix_notifications_or_hidden_placeholders(){
        let runtime=SharedRuntime::new(Config::parse(
            "width=96\nheight=36\nborder-size=0\nmargin=0\nmax-visible=1\n\
             background-color=#224466\n\
             [category=left]\nanchor=bottom-left\nbackground-color=#887755\n\
             [hidden]\nbackground-color=#BB2222\n"
        ).unwrap());
        let now=std::time::Instant::now();
        let root_old=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(root_old,now).unwrap();
        let left=runtime.store.lock().unwrap().push(Request{
            category:"left".into(),..Request::default()
        });
        runtime.schedule_without_hook(left,now).unwrap();
        let root_new=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(root_new,now).unwrap();
        let views=surface_visibility(&runtime).unwrap();
        let root=views.iter().find(|v|v.surface_key.2=="top-right").unwrap();
        let side=views.iter().find(|v|v.surface_key.2=="bottom-left").unwrap();
        let root_frame=snapshot_surface_frame_scaled(&runtime,&root.surface_key,1)
            .unwrap().expect("root surface");
        let side_frame=snapshot_surface_frame_scaled(&runtime,&side.surface_key,1)
            .unwrap().expect("side surface");
        let has=|frame:&PixelFrame,color:[u8;4]|{
            frame.pixels.chunks_exact(4).any(|pixel|pixel==color)
        };
        assert!(has(&root_frame,[0x66,0x44,0x22,0xff]));
        assert!(has(&root_frame,[0x22,0x22,0xBB,0xff]),
            "overflow placeholder belongs to the root surface");
        assert!(!has(&root_frame,[0x55,0x77,0x88,0xff]));
        assert!(has(&side_frame,[0x55,0x77,0x88,0xff]));
        assert!(!has(&side_frame,[0x22,0x22,0xBB,0xff]),
            "other surface's hidden counter cannot create a side placeholder");
        assert!(!has(&side_frame,[0x66,0x44,0x22,0xff]));
        let missing=("DP-NOT-PRESENT".into(),"top".into(),"center".into());
        assert!(snapshot_surface_frame_scaled(&runtime,&missing,1).unwrap().is_none());
    }

    #[test]
    fn surface_visibility_reports_local_hidden_counts_and_totals(){
        let runtime=SharedRuntime::new(Config::parse(
            "max-visible=1\n[category=left]\nanchor=bottom-left\n"
        ).unwrap());
        let now=std::time::Instant::now();
        let first=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(first,now).unwrap();
        let left=runtime.store.lock().unwrap().push(Request{
            category:"left".into(),..Request::default()
        });
        runtime.schedule_without_hook(left,now).unwrap();
        let last=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(last,now).unwrap();
        let views=surface_visibility(&runtime).unwrap();
        assert_eq!(views.len(),2);
        let root=views.iter().find(|v|v.surface_key.2=="top-right").unwrap();
        let side=views.iter().find(|v|v.surface_key.2=="bottom-left").unwrap();
        assert_eq!(root.total_count,2);
        assert_eq!(root.hidden_count,1);
        assert_eq!(root.notification_ids,vec![last]);
        assert_eq!(side.total_count,1);
        assert_eq!(side.hidden_count,0);
        assert_eq!(side.notification_ids,vec![left]);
    }

    #[test]
    fn max_visible_is_counted_per_associated_surface_not_globally(){
        let runtime=SharedRuntime::new(Config::parse(
            "max-visible=1\n[category=left]\nanchor=bottom-left\n"
        ).unwrap());
        let now=std::time::Instant::now();
        let older=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(older,now).unwrap();
        let left=runtime.store.lock().unwrap().push(Request{
            category:"left".into(),..Request::default()
        });
        runtime.schedule_without_hook(left,now).unwrap();
        let newest=runtime.store.lock().unwrap().push(Request::default());
        runtime.schedule_without_hook(newest,now).unwrap();
        let selected=visible_members_indexed(&runtime).unwrap()
            .into_iter().map(|m|m.notification.id).collect::<Vec<_>>();
        assert_eq!(selected,vec![newest,left],
            "independent bottom-left notification must not consume top-right max-visible");
        assert!(!selected.contains(&older));
    }

    #[test]
    fn output_scoped_max_visible_controls_notification_selection(){
        let runtime=SharedRuntime::new(Config::parse(
            "output=DP-1\nmax-visible=1\n[output=DP-1]\nmax-visible=3\n"
        ).unwrap());
        // Mako permits max-visible to be overridden by output or anchor,
        // not by a category selector.
        let older=runtime.store.lock().unwrap().push(Request{
            category:"urgent".into(),..Request::default()
        });
        let middle=runtime.store.lock().unwrap().push(Request{
            category:"urgent".into(),..Request::default()
        });
        let newest=runtime.store.lock().unwrap().push(Request::default());
        // Real Notify assigns a surface to each new notification after the
        // first criteria pass. Exercise that transition before repainting.
        for id in [older,middle,newest] {
            runtime.schedule_without_hook(id,std::time::Instant::now()).unwrap();
        }
        assert_eq!(visible_notifications(&runtime).unwrap().len(),1,
            "configured output is not a compositor assignment");
        runtime.set_surface_output(Some("DP-1")).unwrap();
        let shown=visible_notifications(&runtime).unwrap();
        assert_eq!(shown.iter().map(|n|n.id).collect::<Vec<_>>(),
            vec![newest,middle,older]);
    }

    #[test]
    fn whitespace_prefixed_max_visible_controls_existing_notification_count(){
        let runtime=SharedRuntime::new(Config::parse("max-visible= 1\n").unwrap());
        runtime.store.lock().unwrap().push(Request::default());
        runtime.store.lock().unwrap().push(Request::default());
        assert_eq!(visible_notifications(&runtime).unwrap().len(),1,
            "Mako-validated whitespace must not reset the visibility limit");
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
    fn surviving_stale_group_leader_keeps_its_recorded_group_style() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\n[category=warning]\ngroup-by=app-name\n[grouped]\nbackground-color=#112233\n"
        ).unwrap());
        let now=std::time::Instant::now();
        let old=runtime.store.lock().unwrap().push(Request{
            app_name:"app".into(),tag:"progress".into(),..Request::default()
        });
        runtime.schedule(old,now).unwrap();
        let leader=runtime.store.lock().unwrap().push(Request{
            app_name:"app".into(),category:"warning".into(),..Request::default()
        });
        runtime.schedule(leader,now).unwrap();
        assert_eq!(crate::grouping::active_group_index(&runtime,leader).unwrap(),0);
        assert_eq!(crate::grouping::active_group_index(&runtime,old).unwrap(),1);
        let replaced=runtime.store.lock().unwrap().push(Request{
            app_name:"app".into(),tag:"progress".into(),..Request::default()
        });
        assert_eq!(replaced,old);
        runtime.schedule(replaced,now).unwrap();
        let visible=visible_members_indexed(&runtime).unwrap();
        let first=visible.iter().find(|member|member.notification.id==leader).unwrap();
        assert_eq!((first.group_index,first.group_count),(0,2),
            "Mako preserves the grouping style and original count");
        assert_eq!(crate::grouping::active_group_index(&runtime,leader).unwrap(),0);
        let style=style_for_note_index(&runtime,&first.notification,
            &["default".into()],first.group_index);
        assert_eq!(style["background-color"],"#112233");
    }

    #[test]
    fn scoped_group_by_reaches_visible_projection_and_formatting() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\n[category=warning]\ngroup-by=app-name\n"
        ).unwrap());
        let old=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),category:"success".into(),..Request::default()
        });
        let unrelated=runtime.store.lock().unwrap().push(Request{
            app_name:"other".into(),category:"success".into(),..Request::default()
        });
        let warning=runtime.store.lock().unwrap().push(Request{
            app_name:"same".into(),category:"warning".into(),..Request::default()
        });
        let projection=visible_members_indexed(&runtime).unwrap();
        assert_eq!(projection.len(),2);
        assert_eq!(projection.iter().map(|m|(m.notification.id,m.group_count,m.group_index))
            .collect::<Vec<_>>(),vec![(warning,2,0),(unrelated,1,-1)]);
        assert_eq!(displayable_groups(&runtime).unwrap(),2);
        assert!(projection.iter().all(|m|m.notification.id!=old));
    }

    #[test]
    fn absorbed_scoped_group_member_uses_group_index_style() {
        let runtime=SharedRuntime::new(Config::parse(
            "group-by=none\nmax-visible=4\n[category=warning]\ngroup-by=app-name\n"
        ).unwrap());
        let old=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"success".into(),
            ..Request::default()
        });
        let warning=runtime.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"warning".into(),
            ..Request::default()
        });
        let visible=visible_members_indexed(&runtime).unwrap();
        assert_eq!(visible.len(),1);
        assert_eq!(visible[0].notification.id,warning);
        assert_eq!(crate::grouping::active_group_index(&runtime,old).unwrap(),1);
        let with_unhide=SharedRuntime::new(Config::parse(
            "group-by=none\nmax-visible=4\n[category=warning]\ngroup-by=app-name\n[group-index=1]\ninvisible=0\n"
        ).unwrap());
        let old=with_unhide.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"success".into(),
            ..Request::default()
        });
        let warning=with_unhide.store.lock().unwrap().push(Request{
            app_name:"sender".into(),category:"warning".into(),
            ..Request::default()
        });
        let visible=visible_members_indexed(&with_unhide).unwrap();
        assert_eq!(visible.iter().map(|m|(m.notification.id,m.group_index))
            .collect::<Vec<_>>(),vec![(warning,0),(old,1)]);
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
    fn oversized_hidden_placeholder_does_not_widen_real_notification_canvas(){
        let runtime=SharedRuntime::new(Config::parse(
            "width=100\nheight=40\nborder-size=0\nmargin=4\nmax-visible=1\n\
             [hidden]\nwidth=500\nbackground-color=#BB2222\n"
        ).unwrap());
        let now=std::time::Instant::now();
        for number in 0..2{
            let id=runtime.store.lock().unwrap().push(Request{
                summary:format!("Visible {number}"),..Request::default()
            });
            runtime.schedule_without_hook(id,now).unwrap();
        }
        let frame=snapshot_frame_scaled(&runtime,1).unwrap().unwrap();
        assert_eq!(frame.width,108,
            "Mako render.c excludes the synthetic hidden popup from max_width");
        let double=snapshot_frame_scaled(&runtime,2).unwrap().unwrap();
        assert_eq!(double.width,216);
        assert!(frame.height>40,
            "synthetic placeholder still contributes its own vertical space");
        let hotspots=crate::input::input_rectangles(&runtime,None).unwrap();
        assert_eq!(hotspots.len(),1,
            "the synthetic hidden-count popup is not a clickable notification");
        assert_eq!((hotspots[0].x,hotspots[0].width),(4,100));
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
    fn physical_snapshot_scales_geometry_and_spacing() {
        let runtime=SharedRuntime::new(Config::parse(
            "width=120\nheight=45\nborder-size=0\nmargin=4,6,8,10\n"
        ).unwrap());
        runtime.store.lock().unwrap().push(Request {
            summary:"Readable Pango text".into(),..Request::default()
        });
        let one=snapshot_frame_scaled(&runtime,1).unwrap().unwrap();
        let two=snapshot_frame_scaled(&runtime,2).unwrap().unwrap();
        assert_eq!(two.width,one.width*2);
        // Actual Mako PangoAttrScale font metrics can change logical
        // popup height across outputs; only the margins scale linearly.
        let note=runtime.store.lock().unwrap().active()[0].clone();
        let style=runtime.config().global.clone();
        let painted=render_notification_group_with_icon_scale(&note,&style,0,2,2)
            .unwrap();
        assert_eq!(two.height,painted.height+8,
            "the 4px top margin scales to 8px at output scale 2");
        assert!(snapshot_frame_scaled(&runtime,0).is_err());
    }

    #[test]
    fn uppercase_invisible_rule_excludes_displayable_groups() {
        let runtime=SharedRuntime::new(Config::parse(
            "[category=silent]\ninvisible=TRUE\n"
        ).unwrap());
        runtime.store.lock().unwrap().push(Request{
            category:"silent".into(),..Request::default()
        });
        assert_eq!(displayable_groups(&runtime).unwrap(),0);
        assert!(visible_members_indexed(&runtime).unwrap().is_empty());
    }

    #[test]
    fn empty_snapshot_has_no_surface() {
        let runtime=SharedRuntime::new(Config::default());
        assert!(snapshot_frame(&runtime).unwrap().is_none());
    }
}
