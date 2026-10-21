//! Cairo/Pango software renderer for Mako-like notification surfaces.
//!
//! Renders real Pango markup/text into a premultiplied ARGB32 buffer suitable
//! for Wayland wl_shm on little-endian Linux. Icons, multi-notification layout,
//! scaling and input events remain separate MVP tasks.

use pangocairo::{cairo, pango};
use crate::config::Options;
use crate::format::{decode_format, escape_markup, render_notification_format, FormatNotification};
use crate::notification::Notification;
use crate::raster::Color;
use crate::icons::{IconPixels,load_note_icon};

#[derive(Clone, Debug)]
pub struct PixelFrame {
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    /// Little-endian BGRA, premultiplied alpha (wl_shm ARGB8888).
    pub pixels: Vec<u8>,
}

fn integer_option(style: &Options, key: &str, default: u32) -> u32 {
    style.get(key).and_then(|v| v.parse::<u32>().ok()).unwrap_or(default)
}

fn color_option(style: &Options, key: &str, fallback: &str) -> Result<Color, String> {
    Color::parse_mako(style.get(key).map(String::as_str).unwrap_or(fallback))
        .map_err(|e| format!("{key}: {e}"))
}

fn set_color(cr: &cairo::Context, c: Color) {
    cr.set_source_rgba(f64::from(c.red) / 255.0, f64::from(c.green) / 255.0,
        f64::from(c.blue) / 255.0, f64::from(c.alpha) / 255.0);
}

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

fn paint_icon(cr:&cairo::Context,icon:&IconPixels,x:f64,y:f64,scale:f64)
    -> Result<(),String>
{
    let mut bitmap=cairo::ImageSurface::create(cairo::Format::ARgb32,
        icon.width as i32,icon.height as i32).map_err(|e|e.to_string())?;
    let stride=bitmap.stride() as usize;
    {
        let mut pixels=bitmap.data().map_err(|e|e.to_string())?;
        let row_bytes=icon.width as usize*4;
        for row in 0..icon.height as usize {
            let dest=row*stride;
            let source=row*row_bytes;
            pixels[dest..dest+row_bytes]
                .copy_from_slice(&icon.bgra[source..source+row_bytes]);
        }
    }
    bitmap.mark_dirty();
    cr.save().map_err(|e|e.to_string())?;
    cr.translate(x,y);
    cr.scale(scale,scale);
    let result=(||{
        cr.set_source_surface(&bitmap,0.0,0.0).map_err(|e|e.to_string())?;
        cr.paint().map_err(|e|e.to_string())
    })();
    cr.restore().map_err(|e|e.to_string())?;
    result
}

/// Mako treats `height` as a maximum, not a fixed popup height. Measure
/// the Pango layout before painting so rounded bottom borders are drawn at the
/// actual notification height, instead of cropping a taller buffer.
pub fn measure_notification_height(
    note:&Notification,style:&Options,
) -> Result<u32,String> {
    measure_notification_height_group(note,style,0)
}

pub fn measure_notification_height_group(
    note:&Notification,style:&Options,group_count:usize,
) -> Result<u32,String> {
    let icon=load_note_icon(note,style);
    measure_height_with_icon(note,style,icon.as_ref(),group_count)
}

fn measure_height_with_icon(
    note:&Notification,style:&Options,icon:Option<&IconPixels>,group_count:usize,
) -> Result<u32,String> {
    let width=integer_option(style,"width",300);
    let cap=integer_option(style,"height",100);
    if width==0||cap==0||width>4096||cap>4096{
        return Err("invalid notification dimensions".into());
    }
    let border=integer_option(style,"border-size",2).min(width/2);
    let padding=style.get("padding")
        .and_then(|s|s.split(',').next()).and_then(|s|s.parse::<u32>().ok())
        .unwrap_or(5);
    let frame_padding=border.saturating_add(padding);
    let icon_location=style.get("icon-location").map(String::as_str).unwrap_or("left");
    let vertical=icon_location=="top"||icon_location=="bottom";
    let max_size=integer_option(style,"max-icon-size",64);
    let icon_size=icon.as_ref().map(|img|{
        let ratio=1.0f64.min(f64::from(max_size)/f64::from(img.width.max(img.height)));
        ((f64::from(img.width)*ratio).round() as u32,
         (f64::from(img.height)*ratio).round() as u32)
    });
    let text_width=width.saturating_sub(frame_padding.saturating_mul(2))
        .saturating_sub(
            if !vertical{icon_size.map_or(0,|(w,_)|w.saturating_add(padding.saturating_mul(2)))}
            else{0}
        ).max(1);
    let text_height=cap.saturating_sub(frame_padding.saturating_mul(2))
        .saturating_sub(
            if vertical{icon_size.map_or(0,|(_,h)|h.saturating_add(padding.saturating_mul(2)))}
            else{0}
        ).max(1);

    let markup=style.get("markup")
        .is_none_or(|v|v=="1"||v.eq_ignore_ascii_case("true"));
    let format=style.get("format").map(String::as_str).unwrap_or("<b>%s</b>\\n%b");
    let decoded=decode_format(format).map_err(|e|e.to_string())?;
    let text=render_notification_format(&decoded,&FormatNotification{
        id:note.id,app_name:&note.app_name,summary:&note.summary,body:&note.body,group_count,
    },markup&&pango::parse_markup(&note.body,'\0').is_ok());
    let safe=if pango::parse_markup(&text,'\0').is_ok(){
        text
    }else{
        escape_markup(&format!("{}\\n{}",note.summary,note.body))
    };
    let dummy=cairo::ImageSurface::create(cairo::Format::ARgb32,1,1)
        .map_err(|e|e.to_string())?;
    let context=cairo::Context::new(&dummy).map_err(|e|e.to_string())?;
    let layout=pangocairo::functions::create_layout(&context);
    layout.set_font_description(Some(&pango::FontDescription::from_string(
        style.get("font").map(String::as_str).unwrap_or("monospace 10")
    )));
    layout.set_width((text_width as i32).saturating_mul(pango::SCALE));
    layout.set_height((text_height as i32).saturating_mul(pango::SCALE));
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_markup(&safe);
    let measured=if layout.character_count()>0{
        layout.pixel_size().1.max(0) as u32
    }else{0};
    let text=measured.min(text_height);
    let content=if let Some((_,icon_h))=icon_size{
        if vertical{text.saturating_add(icon_h).saturating_add(padding)}
        else{text.max(icon_h)}
    }else{text};
    let mut actual=content.saturating_add(frame_padding.saturating_mul(2));
    let radius=style.get("border-radius").and_then(|s|s.split(',').next())
        .and_then(|s|s.parse::<u32>().ok()).unwrap_or(0);
    actual=actual.max(radius.saturating_mul(2).saturating_add(border.saturating_mul(2)));
    Ok(actual.min(cap).max(1))
}

pub fn render_notification(note: &Notification, style: &Options) -> Result<PixelFrame, String> {
    render_notification_group(note,style,0)
}

pub fn render_notification_group(
    note:&Notification,style:&Options,group_count:usize,
) -> Result<PixelFrame,String> {
    let width = integer_option(style, "width", 300);
    // One decode per frame. Both text measurement and painting consume the
    // same icon, avoiding redundant filesystem reads and image decoding.
    let icon=load_note_icon(note,style);
    let height = measure_height_with_icon(note,style,icon.as_ref(),group_count)?;
    if width == 0 || height == 0 || width > 4096 || height > 4096
        || u64::from(width) * u64::from(height) * 4 > 64 * 1024 * 1024
    {
        return Err("invalid or oversized notification surface".into());
    }
    let (w, h) = (width as i32, height as i32);
    let border = integer_option(style, "border-size", 2).min(width/2).min(height/2) as f64;
    let radius = style.get("border-radius")
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<u32>().ok()).unwrap_or(0) as f64;
    let padding = style.get("padding")
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<u32>().ok()).unwrap_or(5) as f64;
    let foreground = color_option(style, "text-color", "#FFFFFFFF")?;
    let background = color_option(style, "background-color", "#285577FF")?;
    let border_color = color_option(style, "border-color", "#4C7899FF")?;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .map_err(|e| e.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    context.set_operator(cairo::Operator::Source);
    set_color(&context, border_color);
    rounded(&context, 0.0, 0.0, f64::from(width), f64::from(height), radius);
    context.fill().map_err(|e| e.to_string())?;
    context.set_operator(cairo::Operator::Over);
    if border > 0.0 {
        set_color(&context, background);
        rounded(&context, border, border, f64::from(width) - 2.0*border,
            f64::from(height) - 2.0*border, (radius-border).max(0.0));
        context.fill().map_err(|e| e.to_string())?;
    } else {
        // An unbordered rectangle is entirely the configured background.
        set_color(&context, background);
        rounded(&context, 0.0, 0.0, f64::from(width), f64::from(height), radius);
        context.fill().map_err(|e| e.to_string())?;
    }

    if note.progress>0 {
        let configured=style.get("progress-color").map(String::as_str)
            .unwrap_or("#5588AAFF");
        let (operator,color)=if let Some(v)=configured.strip_prefix("source "){
            (cairo::Operator::Source,v)
        }else if let Some(v)=configured.strip_prefix("over "){
            (cairo::Operator::Over,v)
        }else{
            (cairo::Operator::Over,configured)
        };
        let color=Color::parse_mako(color)
            .map_err(|e|format!("progress-color: {e}"))?;
        let inner_width=(f64::from(width)-2.0*border).max(0.0);
        let inner_height=(f64::from(height)-2.0*border).max(0.0);
        context.save().map_err(|e|e.to_string())?;
        rounded(&context,border,border,inner_width,inner_height,
            (radius-border).max(0.0));
        context.clip();
        context.set_operator(operator);
        set_color(&context,color);
        context.rectangle(border,border,
            inner_width*f64::from(note.progress.min(100))/100.0,inner_height);
        context.fill().map_err(|e|e.to_string())?;
        context.restore().map_err(|e|e.to_string())?;
    }

    let markup = style.get("markup")
        .is_none_or(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let body_is_valid_markup = markup && pango::parse_markup(&note.body, '\0').is_ok();
    let format = style.get("format").map(String::as_str).unwrap_or("<b>%s</b>\\n%b");
    let decoded = decode_format(format).map_err(|e| e.to_string())?;
    let rendered = render_notification_format(&decoded, &FormatNotification {
        id: note.id, app_name: &note.app_name, summary: &note.summary,
        body: &note.body, group_count,
    }, body_is_valid_markup);
    // If the configured template is malformed, escape *everything* instead of
    // interpreting unintended markup from a remote notification sender.
    let safe_markup = if pango::parse_markup(&rendered, '\0').is_ok() {
        rendered
    } else {
        escape_markup(&format!("{}\\n{}", note.summary, note.body))
    };

    let interior_w=(f64::from(width)-(border+padding)*2.0).max(1.0);
    let interior_h=(f64::from(height)-(border+padding)*2.0).max(1.0);
    let mut text_x=border+padding;
    let mut text_y=border+padding;
    let mut available_w=interior_w;
    let mut available_h=interior_h;

    if let Some(icon)=icon.as_ref() {
        let location=style.get("icon-location").map(String::as_str).unwrap_or("left");
        let max_size=f64::from(integer_option(style,"max-icon-size",64));
        let w=f64::from(icon.width);
        let h=f64::from(icon.height);
        let horizontal=location=="left"||location=="right";
        let reserved=if horizontal{available_w}else{available_h};
        let max_length=(reserved-padding-1.0).max(0.0);
        let scale=1.0f64.min(max_size/w.max(h))
            .min(interior_w/w).min(interior_h/h)
            .min(max_length/if horizontal{w}else{h});
        if scale>0.0{
            let (iw,ih)=(w*scale,h*scale);
            let x=match location {
                "right"=>f64::from(width)-border-padding-iw,
                "top"|"bottom"=>border+padding+(interior_w-iw)/2.0,
                _=>border+padding,
            };
            let y=match location {
                "bottom"=>f64::from(height)-border-padding-ih,
                "left"|"right"=>border+padding+(interior_h-ih)/2.0,
                _=>border+padding,
            };
            paint_icon(&context,&icon,x,y,scale)?;
            match location {
                "right"=>available_w=(available_w-iw-padding).max(1.0),
                "top"=>{text_y+=ih+padding;available_h=(available_h-ih-padding).max(1.0)},
                "bottom"=>available_h=(available_h-ih-padding).max(1.0),
                _=>{text_x+=iw+padding;available_w=(available_w-iw-padding).max(1.0)},
            }
        }
    }

    context.rectangle(text_x, text_y, available_w, available_h);
    context.clip();
    context.move_to(text_x, text_y);
    set_color(&context, foreground);
    let layout = pangocairo::functions::create_layout(&context);
    let font = style.get("font").map(String::as_str).unwrap_or("monospace 10");
    layout.set_font_description(Some(&pango::FontDescription::from_string(font)));
    layout.set_width((available_w as i32).saturating_mul(pango::SCALE));
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_height((available_h as i32).saturating_mul(pango::SCALE));
    layout.set_ellipsize(pango::EllipsizeMode::End);
    match style.get("text-alignment").map(String::as_str) {
        Some("center")=>layout.set_alignment(pango::Alignment::Center),
        Some("right")=>layout.set_alignment(pango::Alignment::Right),
        _=>layout.set_alignment(pango::Alignment::Left),
    }
    layout.set_markup(&safe_markup);
    pangocairo::functions::show_layout(&context, &layout);
    drop(layout);
    drop(context);
    surface.flush();
    let stride = surface.stride();
    let mut pixels = Vec::new();
    surface.with_data(|data| pixels.extend_from_slice(data))
        .map_err(|e| e.to_string())?;
    Ok(PixelFrame {width, height, stride, pixels})
}


/// Compose ordered Mako-style popup frames into a single layer-shell surface.
/// Each frame is already rendered and has an independent background. The
/// transparent gaps are intentionally *not* colored by the compositor.
/// Paint Mako's synthetic '(N more)' notification, retaining the style of
/// [hidden] criteria and the %h/%t format substitutions. This is not stored
/// in notification history or exposed as a real D-Bus notification.
pub fn render_hidden_placeholder(
    hidden:usize,total:usize,style:&Options,
) -> Result<PixelFrame,String> {
    let raw=style.get("format").map(String::as_str).unwrap_or("(%h more)");
    let decoded=decode_format(raw).map_err(|e|e.to_string())?;
    let literal=crate::format::render_hidden_format(&decoded,hidden,total);
    // The ordinary render path parses the format a second time; preserve
    // literal percent and backslashes in the computed placeholder.
    let encoded=literal.replace('\\',"\\\\").replace('%',"%%");
    let mut prepared=style.clone();
    prepared.insert("format".into(),encoded);
    prepared.insert("icons".into(),"0".into());
    let mut store=crate::notification::NotificationStore::new(0);
    store.push(crate::notification::Request::default());
    let note=store.active().front().ok_or("failed to create hidden placeholder")?;
    render_notification(note,&prepared)
}

pub fn compose_stack(frames: &[PixelFrame], gap: u32) -> Result<Option<PixelFrame>, String> {
    compose_stack_aligned(frames,gap,StackAlignment::Left)
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum StackAlignment { Left,Center,Right }

pub fn compose_stack_aligned(
    frames: &[PixelFrame], gap: u32, alignment: StackAlignment,
) -> Result<Option<PixelFrame>, String> {
    let gaps=vec![gap;frames.len().saturating_sub(1)];
    compose_stack_spaced(frames,&gaps,alignment)
}

/// Honor distinct adjacent Mako top/bottom margins between frames. Callers
/// calculate each gap as max(previous.bottom, next.top).
pub fn compose_stack_spaced(
    frames: &[PixelFrame],gaps:&[u32],alignment:StackAlignment,
) -> Result<Option<PixelFrame>,String> {
    if frames.is_empty(){return Ok(None);}
    if gaps.len()!=frames.len()-1{return Err("invalid popup gap count".into());}
    let width=frames.iter().map(|f|f.width).max().unwrap_or(0);
    let content_height:u64=frames.iter().map(|f|u64::from(f.height)).sum();
    let spaces:u64=gaps.iter().map(|g|u64::from(*g)).sum();
    let height=content_height.checked_add(spaces)
        .ok_or("popup stack overflow")?;
    let stride=u64::from(width).checked_mul(4).ok_or("popup stack stride overflow")?;
    let size=stride.checked_mul(height).ok_or("popup stack buffer overflow")?;
    if width==0 || height==0 || height>4096 || width>4096 || size>64*1024*1024 {
        return Err("popup stack exceeds safe Wayland bounds".into());
    }
    let mut pixels=vec![0;size as usize];
    let mut offset_y:usize=0;
    for (index,frame) in frames.iter().enumerate() {
        if frame.height==0 || frame.width==0 ||
           frame.stride<i32::try_from(frame.width*4).map_err(|_|"invalid frame stride")? ||
           frame.pixels.len() != (frame.stride as usize)*(frame.height as usize) {
            return Err("invalid child frame".into());
        }
        let row_bytes=frame.width as usize * 4;
        let horizontal_offset=match alignment {
            StackAlignment::Left=>0,
            StackAlignment::Center=>(width-frame.width)/2,
            StackAlignment::Right=>width-frame.width,
        } as usize*4;
        for y in 0..frame.height as usize {
            let src=y*frame.stride as usize;
            let dst=(offset_y+y)*stride as usize+horizontal_offset;
            pixels[dst..dst+row_bytes].copy_from_slice(&frame.pixels[src..src+row_bytes]);
        }
        offset_y+=frame.height as usize
            +gaps.get(index).copied().unwrap_or(0) as usize;
    }
    Ok(Some(PixelFrame{width,height:height as u32,stride:stride as i32,pixels}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::{Request, NotificationStore};

    fn fixture() -> Notification {
        let mut store = NotificationStore::new(5);
        let id = store.push(Request { summary: "Text visible".into(),
            body: "Line 2".into(), category: "success".into(),
            ..Request::default() });
        store.active().iter().find(|n| n.id == id).unwrap().clone()
    }

    #[test]
    fn shrink_wrapped_notification_is_shorter_than_max_height() {
        let mut style=Options::new();
        for (key,value) in [("width","360"),("height","160"),
            ("font","Sans 11"),("border-size","2"),("padding","6")]{
            style.insert(key.into(),value.into());
        }
        let frame=render_notification(&fixture(),&style).unwrap();
        assert!(frame.height<160, "short text should not fill Mako max-height");
        assert!(frame.height>=20, "short text should still have a readable frame");
    }

    #[test]
    fn multiline_and_long_notifications_measure_differently() {
        let mut one=fixture();
        one.summary="Short".into();
        one.body="".into();
        let mut multi=fixture();
        multi.summary="Short".into();
        multi.body="A\\nB\\nC\\nD".into();
        let mut style=Options::new();
        style.insert("width".into(),"160".into());
        style.insert("height".into(),"140".into());
        let first=render_notification(&one,&style).unwrap();
        let second=render_notification(&multi,&style).unwrap();
        assert!(second.height>first.height);
        assert!(second.height<=140);
    }

    #[test]
    fn inline_dbus_image_pixels_take_priority_over_app_icon_file() {
        let mut note=fixture();
        note.app_icon="/definitely/missing.png".into();
        note.image_data=crate::icons::decode_raw_image(
            8,8,32,true,8,4,&[255,0,0,255].repeat(64));
        let mut style=Options::new();
        for (key,value) in [
            ("width","200"),("height","100"),("border-size","0"),
            ("padding","4"),("icons","1"),("icon-location","left"),
            ("background-color","#305747"),
        ]{
            style.insert(key.into(),value.into());
        }
        let frame=render_notification(&note,&style).unwrap();
        let offset=(frame.height as usize/2)*frame.stride as usize+6*4;
        assert_eq!(&frame.pixels[offset..offset+4],&[0,0,255,255]);
    }

    #[test]
    fn draws_a_real_icon_in_left_slot_and_respects_icons_disabled() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("synthetic.png");
        let pixbuf=gdk_pixbuf::Pixbuf::new(
            gdk_pixbuf::Colorspace::Rgb,true,8,12,12
        ).unwrap();
        pixbuf.fill(0xFF0000FF);
        pixbuf.savev(&path,"png",&[]).unwrap();
        let mut note=fixture();
        note.app_icon=path.to_string_lossy().into_owned();
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","70"),("padding","4"),
            ("border-size","0"),("icon-location","left"),
            ("max-icon-size","16"),("background-color","#305747"),
        ] {
            style.insert(key.into(),value.into());
        }
        let with=render_notification(&note,&style).unwrap();
        let at=(with.height as usize/2)*with.stride as usize+6*4;
        assert_eq!(&with.pixels[at..at+4],&[0x00,0x00,0xFF,0xFF]);
        style.insert("icons".into(),"0".into());
        let without=render_notification(&note,&style).unwrap();
        // The text moves into the freed icon slot; this point may contain a
        // text glyph rather than background after icons are disabled.
        assert_ne!(&without.pixels[at..at+4],&[0x00,0x00,0xFF,0xFF]);
    }

    #[test]
    fn progress_fill_respects_value_and_uses_configured_color() {
        let mut note=fixture();
        note.progress=50;
        let mut style=Options::new();
        for (key,value) in [
            ("width","120"),("height","90"),("border-size","0"),
            ("background-color","#000000"),("progress-color","#FF0000"),
        ]{style.insert(key.into(),value.into());}
        let frame=render_notification(&note,&style).unwrap();
        let y=frame.height as usize-3;
        let first=y*frame.stride as usize+10*4;
        let last=y*frame.stride as usize+110*4;
        assert_eq!(&frame.pixels[first..first+4],&[0,0,255,255]);
        assert_eq!(&frame.pixels[last..last+4],&[0,0,0,255]);
        note.progress=0;
        let empty=render_notification(&note,&style).unwrap();
        assert_eq!(&empty.pixels[first..first+4],&[0,0,0,255]);
    }

    #[test]
    fn group_count_substitutions_affect_pango_measurement_and_rendering() {
        let mut note=fixture();
        note.summary="Task".into();
        note.body="".into();
        let mut style=Options::new();
        style.insert("format".into(),"(%g) <b>%s</b>".into());
        style.insert("width".into(),"160".into());
        style.insert("height".into(),"80".into());
        let frame=render_notification_group(&note,&style,3).unwrap();
        let cap=measure_notification_height_group(&note,&style,3).unwrap();
        assert_eq!(frame.height,cap);
        assert!(frame.height>0);
    }

    #[test]
    fn hidden_placeholder_uses_own_format_and_shrink_wraps() {
        let mut style=Options::new();
        style.insert("width".into(),"180".into());
        style.insert("height".into(),"100".into());
        style.insert("format".into(),"<b>(%h of %t more)</b>".into());
        let image=render_hidden_placeholder(3,8,&style).unwrap();
        assert_eq!(image.width,180);
        assert!(image.height<100);
        assert!(image.pixels.iter().any(|x|*x!=0));
    }

    #[test]
    fn per_notification_gaps_are_respected_independently() {
        let one=PixelFrame{width:1,height:1,stride:4,pixels:vec![9;4]};
        let result=compose_stack_spaced(
            &[one.clone(),one.clone(),one], &[2,5],StackAlignment::Left
        ).unwrap().unwrap();
        assert_eq!(result.height,10);
        assert_eq!(&result.pixels[0..4],&[9;4]);
        assert_eq!(&result.pixels[4..12],&[0;8]);
        assert_eq!(&result.pixels[12..16],&[9;4]);
        assert_eq!(&result.pixels[16..36],&[0;20]);
        assert_eq!(&result.pixels[36..40],&[9;4]);
        assert!(compose_stack_spaced(
            &[PixelFrame{width:1,height:1,stride:4,pixels:vec![0;4]}],
            &[2],StackAlignment::Left
        ).is_err());
    }

    #[test]
    fn narrower_frames_align_to_the_selected_stack_edge() {
        let wide=PixelFrame{width:4,height:1,stride:16,pixels:vec![1;16]};
        let narrow=PixelFrame{width:2,height:1,stride:8,pixels:vec![2;8]};
        let right=compose_stack_aligned(
            &[wide.clone(),narrow.clone()],0,StackAlignment::Right
        ).unwrap().unwrap();
        assert_eq!(&right.pixels[16..24],&[0;8]);
        assert_eq!(&right.pixels[24..32],&[2;8]);
        let centered=compose_stack_aligned(
            &[wide.clone(),narrow.clone()],0,StackAlignment::Center
        ).unwrap().unwrap();
        assert_eq!(&centered.pixels[16..20],&[0;4]);
        assert_eq!(&centered.pixels[20..28],&[2;8]);
        assert_eq!(&centered.pixels[28..32],&[0;4]);
        let left=compose_stack(&[wide,narrow],0).unwrap().unwrap();
        assert_eq!(&left.pixels[16..24],&[2;8]);
    }

    #[test]
    fn stack_preserves_order_and_transparent_spacers() {
        let first=PixelFrame{width:2,height:1,stride:8,pixels:vec![1;8]};
        let second=PixelFrame{width:1,height:2,stride:4,pixels:vec![2;8]};
        let combined=compose_stack(&[first,second],1).unwrap().unwrap();
        assert_eq!((combined.width,combined.height,combined.stride),(2,4,8));
        assert_eq!(&combined.pixels[0..8],&[1;8]);
        assert_eq!(&combined.pixels[8..16],&[0;8]);
        assert_eq!(&combined.pixels[16..20],&[2;4]);
        assert_eq!(&combined.pixels[20..24],&[0;4]);
        assert_eq!(&combined.pixels[24..28],&[2;4]);
    }

    #[test]
    fn empty_and_oversized_stacks_are_rejected_safely() {
        assert!(compose_stack(&[],10).unwrap().is_none());
        let huge=PixelFrame{width:1,height:4096,stride:4,pixels:vec![0;16384]};
        assert!(compose_stack(&[huge.clone(),huge],4).is_err());
    }

    #[test]
    fn produces_a_pango_raster_with_colored_background() {
        let mut style = Options::new();
        style.insert("width".into(), "280".into());
        style.insert("height".into(), "110".into());
        style.insert("border-size".into(), "2".into());
        style.insert("border-color".into(), "#FFFFFF".into());
        style.insert("background-color".into(), "#305747".into());
        style.insert("font".into(), "Sans 13".into());
        let pixels = render_notification(&fixture(), &style).unwrap();
        assert_eq!(pixels.width,280);
        assert!(pixels.height<110);
        assert!(pixels.height>20);
        assert_eq!(pixels.stride,1120);
        let bg = ((pixels.height as usize-5)*pixels.stride as usize + 100*4) as usize;
        assert_eq!(&pixels.pixels[bg..bg+4], &[0x47,0x57,0x30,0xff]);
        assert_eq!(&pixels.pixels[0..4], &[0xff,0xff,0xff,0xff]);
        assert!(pixels.pixels.chunks_exact(4).any(|p| p[3] > 0 && p[0] != 0x47 && p[0] != 0xff));
    }

    #[test]
    fn malformed_markup_is_not_interpreted() {
        let mut note = fixture();
        note.summary = "<bad>&".into();
        note.body = "<span>unfinished".into();
        let style = Options::new();
        let result = render_notification(&note, &style);
        assert!(result.is_ok());
    }

    #[test]
    fn absurd_surfaces_rejected_before_allocating() {
        let mut style = Options::new();
        style.insert("width".into(), "50000".into());
        assert!(render_notification(&fixture(), &style).is_err());
    }
}
