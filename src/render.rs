//! Cairo/Pango software renderer for Mako-like notification surfaces.
//!
//! Renders Pango markup, icons, grouped counts and overflow text into a
//! premultiplied ARGB32 buffer for Wayland wl_shm on little-endian Linux.
//! Exact upstream Pango metrics and fractional-scale presentation remain MVP tasks.

use pangocairo::{cairo, pango};
use crate::config::Options;
use crate::format::{decode_format, render_notification_format, FormatNotification};
use crate::notification::Notification;
use crate::raster::{Color,ProgressBlend,parse_progress_color_mako};
use crate::icons::{IconPixels,load_note_icon_scaled};

#[derive(Clone, Debug)]
pub struct PixelFrame {
    pub width: u32,
    pub height: u32,
    pub stride: i32,
    /// Little-endian BGRA, premultiplied alpha (wl_shm ARGB8888).
    pub pixels: Vec<u8>,
}

fn integer_option(style: &Options, key: &str, default: u32) -> u32 {
    style.get(key).and_then(|v|crate::config::parse_nonnegative_mako(v))
        .unwrap_or(default)
}

#[cfg(test)]
#[test]
fn validated_leading_whitespace_width_and_border_size_reach_renderer(){
    let mut style=Options::new();
    style.insert("width".into()," 440".into());
    style.insert("border-size".into()," \t+3".into());
    assert_eq!(integer_option(&style,"width",300),440);
    assert_eq!(integer_option(&style,"border-size",2),3);
}

fn color_option(style: &Options, key: &str, fallback: &str) -> Result<Color, String> {
    Color::parse_mako(style.get(key).map(String::as_str).unwrap_or(fallback))
        .map_err(|e| format!("{key}: {e}"))
}

/// Mako's render_notification() falls back to pango_layout_set_text()
/// on malformed *final* template markup. It does not discard the template
/// and reconstruct summary/body. Keep measurement and painting identical.
fn set_mako_formatted_text(layout:&pango::Layout,rendered:&str){
    if pango::parse_markup(rendered,'\0').is_ok(){
        layout.set_markup(rendered);
    }else{
        layout.set_text(rendered);
    }
}

fn set_color(cr: &cairo::Context, c: Color) {
    cr.set_source_rgba(f64::from(c.red) / 255.0, f64::from(c.green) / 255.0,
        f64::from(c.blue) / 255.0, f64::from(c.alpha) / 255.0);
}

#[derive(Clone,Copy,Debug,PartialEq)]
struct CornerRadii {tl:f64,tr:f64,br:f64,bl:f64}
impl CornerRadii {
    fn parse(style:&Options)->Self{
        // Mako uses CSS-like 1..4 values in top-left, top-right,
        // bottom-right, bottom-left order.
        let raw=style.get("border-radius").map(String::as_str).unwrap_or("0");
        let margins=crate::wayland::PopupMargins::parse_mako(raw)
            .unwrap_or(crate::wayland::PopupMargins::uniform(0));
        Self {
            tl:f64::from(margins.top.max(0)),
            tr:f64::from(margins.right.max(0)),
            br:f64::from(margins.bottom.max(0)),
            bl:f64::from(margins.left.max(0)),
        }
    }
    fn required_height(self,border:u32)->u32{
        let corner=self.tl+self.bl;
        let other=self.tr+self.br;
        (corner.max(other) as u32).saturating_add(border.saturating_mul(2))
    }
}
fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: CornerRadii) {
    // Mako's set_rounded_rectangle passes the original radii to Cairo.
    // Clamping to half the rectangle changes large-radius borders/icons.
    // The style parser already prevents negative radii reaching this path.
    let [tl,tr,br,bl]=[radius.tl,radius.tr,radius.br,radius.bl];
    cr.new_sub_path();
    cr.arc(x+w-tr,y+tr,tr,-std::f64::consts::FRAC_PI_2,0.0);
    cr.arc(x+w-br,y+h-br,br,0.0,std::f64::consts::FRAC_PI_2);
    cr.arc(x+bl,y+h-bl,bl,std::f64::consts::FRAC_PI_2,std::f64::consts::PI);
    cr.arc(x+tl,y+tl,tl,std::f64::consts::PI,3.0*std::f64::consts::FRAC_PI_2);
    cr.close_path();
}

/// Pinned Mako v1.11.0 accepts signed padding on all four sides.
/// Its render.c uses these values directly in its Pango layout dimensions,
/// icon offsets and notification height. Clamping negatives to zero makes
/// legitimate Mako configurations display differently.
#[derive(Clone,Copy,Debug)]
struct Padding {top:i32,right:i32,bottom:i32,left:i32}
impl Padding {
    fn parse(style:&Options)->Self{
        let raw=style.get("padding").map(String::as_str).unwrap_or("5");
        let margin=crate::wayland::PopupMargins::parse_mako(raw)
            .unwrap_or(crate::wayland::PopupMargins::uniform(5));
        Self{
            top:margin.top,
            right:margin.right,
            bottom:margin.bottom,
            left:margin.left,
        }
    }
}

fn paint_icon(
    cr:&cairo::Context,icon:&IconPixels,x:f64,y:f64,scale:f64,
    logical_width:u32,logical_height:u32,border_radius:f64,
)->Result<(),String>
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
    // Upstream Mako clips the icon to icon-border-radius before painting.
    // Radius is configured in rendered pixels, so translate it back through
    // the icon's local scaling transform before constructing the clip path.
    let r=(border_radius/scale).max(0.0);
    // icon.c truncates (int)(source_size * icon->scale) before
    // render.c constructs its clipping rectangle. Cairo still paints the
    // source at the full fractional scale inside that integer-sized clip.
    rounded(cr,0.0,0.0,
        f64::from(logical_width)/scale,f64::from(logical_height)/scale,
        CornerRadii{tl:r,tr:r,br:r,bl:r});
    cr.clip();
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
    measure_notification_height_group_with_icon_scale(note,style,group_count,1)
}

/// Match the asset chosen by the frame renderer: Mako resolves a themed
/// icon against the largest attached output scale, even when painting a
/// popup on a smaller-scale monitor. Different theme assets may have
/// different aspect ratios, changing Pango shrink-wrapped popup height.
pub fn measure_notification_height_group_with_icon_scale(
    note:&Notification,style:&Options,group_count:usize,icon_scale:u32,
)->Result<u32,String>{
    let icon=load_note_icon_scaled(note,style,icon_scale.max(1));
    measure_height_with_icon(note,style,icon.as_ref(),group_count)
}

fn icon_fit_limit(note:&Notification,style:&Options)->u32{
    note.icon_snapshot.as_ref().and_then(Option::as_ref)
        .map(|icon|icon.max_size)
        .unwrap_or_else(||integer_option(style,"max-icon-size",64))
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
    let padding=Padding::parse(style);
    let width_padding=i64::from(border)*2+i64::from(padding.left)+i64::from(padding.right);
    let height_padding=i64::from(border)*2+i64::from(padding.top)+i64::from(padding.bottom);
    let icon_location=style.get("icon-location").map(String::as_str).unwrap_or("left");
    let vertical=icon_location=="top"||icon_location=="bottom";
    let max_size=icon_fit_limit(note,style);
    let icon_size=icon.as_ref().map(|img|{
        let ratio=1.0f64.min(f64::from(max_size)/f64::from(img.width.max(img.height)));
        ((f64::from(img.width)*ratio) as u32,
         (f64::from(img.height)*ratio) as u32)
    });
    let horizontal_extra=if icon_location=="right" {padding.right}
        else{padding.left};
    let vertical_extra=if icon_location=="bottom" {padding.bottom}
        else{padding.top};
    let text_width=(i64::from(width)-width_padding-if !vertical {
        icon_size.map_or(0,|(w,_)|i64::from(w)+i64::from(horizontal_extra)*2)
    }else{0}).clamp(1,4096) as i32;
    let text_height=(i64::from(cap)-height_padding-if vertical {
        icon_size.map_or(0,|(_,h)|i64::from(h)+i64::from(vertical_extra)*2)
    }else{0}).clamp(1,4096) as i32;

    let markup=style.get("markup")
        .is_none_or(|v|v=="1"||v.eq_ignore_ascii_case("true"));
    let format=style.get("format").map(String::as_str).unwrap_or("<b>%s</b>\\n%b");
    let decoded=decode_format(format).map_err(|e|e.to_string())?;
    let text=render_notification_format(&decoded,&FormatNotification{
        id:note.id,app_name:&note.app_name,summary:&note.summary,body:&note.body,group_count,
    },markup&&pango::parse_markup(&note.body,'\0').is_ok());
    let dummy=cairo::ImageSurface::create(cairo::Format::ARgb32,1,1)
        .map_err(|e|e.to_string())?;
    let context=cairo::Context::new(&dummy).map_err(|e|e.to_string())?;
    let layout=pangocairo::functions::create_layout(&context);
    layout.set_font_description(Some(&pango::FontDescription::from_string(
        style.get("font").map(String::as_str).unwrap_or("monospace 10")
    )));
    layout.set_width(text_width.saturating_mul(pango::SCALE));
    layout.set_height(text_height.saturating_mul(pango::SCALE));
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    set_mako_formatted_text(&layout,&text);
    let measured=if layout.character_count()>0{
        layout.pixel_size().1.max(0) as u32
    }else{0};
    let text=i64::from(measured).min(i64::from(text_height));
    let content=if let Some((_,icon_h))=icon_size{
        if vertical{text+i64::from(icon_h)+i64::from(vertical_extra)}
        else{text.max(i64::from(icon_h))}
    }else{text};
    // Mako uses signed padding both inside its Pango layout and when
    // adding the notification's surrounding insets. Keep the arithmetic
    // signed until after checking the safe physical popup bounds.
    let actual=(content+height_padding)
        .max(i64::from(CornerRadii::parse(style).required_height(border)))
        .max(1);
    if actual>4096{return Err("notification exceeds safe height".into());}
    Ok(actual as u32)
}

pub fn render_notification(note: &Notification, style: &Options) -> Result<PixelFrame, String> {
    render_notification_group(note,style,0)
}

pub fn render_notification_group(
    note:&Notification,style:&Options,group_count:usize,
) -> Result<PixelFrame,String> {
    render_notification_group_scaled(note,style,group_count,1)
}

/// Draw directly at physical resolution rather than enlarging low-resolution
/// Pango glyph bitmaps. All Mako geometry remains in logical coordinates.
pub fn render_notification_group_scaled(
    note:&Notification,style:&Options,group_count:usize,pixel_scale:u32,
) -> Result<PixelFrame,String> {
    render_notification_group_with_icon_scale(
        note,style,group_count,pixel_scale,pixel_scale
    )
}

/// Mako selects themed icons using the largest scale on *any* attached output,
/// not necessarily the output presenting the popup. Painting and text still
/// rasterize at the actual popup output's pixel scale.
pub fn render_notification_group_with_icon_scale(
    note:&Notification,style:&Options,group_count:usize,
    pixel_scale:u32,icon_lookup_scale:u32,
)->Result<PixelFrame,String>{
    if pixel_scale==0 {
        return Err("invalid notification pixel scale".into());
    }
    let width = integer_option(style, "width", 300);
    // One decode per frame. Both text measurement and painting consume the
    // same icon, avoiding redundant filesystem reads and image decoding.
    let icon=load_note_icon_scaled(note,style,icon_lookup_scale.max(1));
    let height = measure_height_with_icon(note,style,icon.as_ref(),group_count)?;
    if width == 0 || height == 0 || width > 4096 || height > 4096
        || u64::from(width) * u64::from(height) * 4 > 64 * 1024 * 1024
    {
        return Err("invalid or oversized notification surface".into());
    }
    let physical_width=width.checked_mul(pixel_scale).ok_or("physical width overflow")?;
    let physical_height=height.checked_mul(pixel_scale).ok_or("physical height overflow")?;
    if physical_width>4096 || physical_height>4096 ||
        u64::from(physical_width)*u64::from(physical_height)*4>64*1024*1024 {
        return Err("HiDPI notification exceeds safe buffer bounds".into());
    }
    let (w, h) = (physical_width as i32, physical_height as i32);
    let border = integer_option(style, "border-size", 2).min(width/2).min(height/2) as f64;
    let radius=CornerRadii::parse(style);
    let pad=Padding::parse(style);
    let top=f64::from(pad.top);
    let right=f64::from(pad.right);
    let bottom=f64::from(pad.bottom);
    let left=f64::from(pad.left);
    let foreground = color_option(style, "text-color", "#FFFFFFFF")?;
    let background = color_option(style, "background-color", "#285577FF")?;
    let border_color = color_option(style, "border-color", "#4C7899FF")?;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h)
        .map_err(|e| e.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    context.scale(f64::from(pixel_scale),f64::from(pixel_scale));
    // Mako render.c:215-272 draws the background on the CENTERLINE of
    // a stroked border: x=b/2, width=w-b, with unmodified corner radii.
    // Drawing a filled outer border then an inset rounded background
    // changes the corner geometry and alpha blending of translucent borders.
    rounded(&context,border/2.0,border/2.0,
        f64::from(width)-border,f64::from(height)-border,radius);
    set_color(&context,background);
    context.fill_preserve().map_err(|e|e.to_string())?;
    let border_path=context.copy_path().map_err(|e|e.to_string())?;
    if note.progress>0 {
        let configured=style.get("progress-color").map(String::as_str)
            .unwrap_or("#5588AAFF");
        let (blend,color)=parse_progress_color_mako(configured)
            .map_err(|e|format!("progress-color: {e}"))?;
        let operator=match blend{
            ProgressBlend::Over=>cairo::Operator::Over,
            ProgressBlend::Source=>cairo::Operator::Source,
        };
        context.save().map_err(|e|e.to_string())?;
        // Upstream progress is clipped by the original background path,
        // not by a newly constructed inset with reduced corner radii.
        context.clip();
        context.set_operator(operator);
        set_color(&context,color);
        let logical_width=i64::from(width).saturating_sub(2*(border as i64));
        let progress_width=logical_width.saturating_mul(i64::from(note.progress.min(100)))
            / 100;
        // render.c uses y=b and height=notification_height-b. The
        // background-path clip and subsequent border stroke bound it.
        context.rectangle(border,border,progress_width as f64,
            f64::from(height)-border);
        context.fill().map_err(|e|e.to_string())?;
        context.restore().map_err(|e|e.to_string())?;
    }
    // Upstream paints the border LAST with SOURCE, so translucent border
    // pixels replace the prior background/progress instead of mixing with it.
    context.new_path();
    context.append_path(&border_path);
    context.set_operator(cairo::Operator::Source);
    set_color(&context,border_color);
    context.set_line_width(border);
    context.stroke().map_err(|e|e.to_string())?;
    context.set_operator(cairo::Operator::Over);

    let markup = style.get("markup")
        .is_none_or(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let body_is_valid_markup = markup && pango::parse_markup(&note.body, '\0').is_ok();
    let format = style.get("format").map(String::as_str).unwrap_or("<b>%s</b>\\n%b");
    let decoded = decode_format(format).map_err(|e| e.to_string())?;
    let rendered = render_notification_format(&decoded, &FormatNotification {
        id: note.id, app_name: &note.app_name, summary: &note.summary,
        body: &note.body, group_count,
    }, body_is_valid_markup);
    // Upstream treats invalid final markup as literal *formatted* text,
    // preserving template prefixes, suffixes and field substitutions.

    let interior_w=(f64::from(width)-2.0*border-left-right).max(1.0);
    let interior_h=(f64::from(height)-2.0*border-top-bottom).max(1.0);
    let mut text_x=border+left;
    let mut text_y=border+top;
    let mut available_w=interior_w;
    let mut available_h=interior_h;

    if let Some(icon)=icon.as_ref() {
        let location=style.get("icon-location").map(String::as_str).unwrap_or("left");
        let max_size=f64::from(icon_fit_limit(note,style));
        let w=f64::from(icon.width);
        let h=f64::from(icon.height);
        // Mako icon.c scales ONLY against max-icon-size, then truncates
        // logical icon dimensions. It does not shrink the icon a second
        // time to fit text space or the notification's measured height.
        let scale=1.0f64.min(max_size/w.max(h));
        if scale>0.0{
            let (iw,ih)=((w*scale).trunc(),(h*scale).trunc());
            let x=match location {
                "right"=>f64::from(width)-border-right-iw,
                "top"|"bottom"=>(f64::from(width)-iw)/2.0,
                _=>border+left,
            };
            let y=match location {
                "bottom"=>f64::from(height)-border-bottom-ih,
                "left"|"right"=>(f64::from(height)-ih)/2.0,
                _=>border+top,
            };
            let icon_radius=integer_option(style,"icon-border-radius",0) as f64;
            paint_icon(&context,&icon,x,y,scale,iw as u32,ih as u32,icon_radius)?;
            match location {
                "right"=>available_w=(available_w-iw-2.0*right).max(1.0),
                "top"=>{text_y+=ih+top;available_h=(available_h-ih-2.0*top).max(1.0)},
                "bottom"=>available_h=(available_h-ih-2.0*bottom).max(1.0),
                _=>{text_x+=iw+left;available_w=(available_w-iw-2.0*left).max(1.0)},
            }
        }
    }

    // Pinned Mako v1.11.0 render.c centers text vertically for left/right
    // icon locations (including no icon), and horizontally for top/bottom.
    // Measure the Pango ink box before positioning: the popup's height may
    // be determined by a tall icon rather than by its one-line text.
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
    set_mako_formatted_text(&layout,&rendered);
    let (text_width,text_height)=if layout.character_count()>0{
        layout.pixel_size()
    }else{(0,0)};
    let vertical_icon=matches!(style.get("icon-location").map(String::as_str),
        Some("top"|"bottom"));
    // Mako render.c defines local border_size=2*style->border_size.
    // It centers inside (size-2*border) then adds border to move_to,
    // yielding exact centering over the whole notification surface.
    if vertical_icon {
        text_x=(f64::from(width)-f64::from(text_width))/2.0;
    }else{
        text_y=(f64::from(height)-f64::from(text_height))/2.0;
    }
    // Mako render.c:318-325 draws the Pango layout directly after setting
    // its width/height and ellipsizing. It does NOT add another Cairo clip.
    // The old extra clip could erase glyph ink outside the logical layout
    // rectangle (e.g. italic overhangs), even though Mako draws that ink.
    context.move_to(text_x,text_y);
    set_color(&context,foreground);
    pangocairo::functions::show_layout(&context, &layout);
    drop(layout);
    drop(context);
    surface.flush();
    let stride = surface.stride();
    let mut pixels = Vec::new();
    surface.with_data(|data| pixels.extend_from_slice(data))
        .map_err(|e| e.to_string())?;
    Ok(PixelFrame {width:physical_width,height:physical_height,stride,pixels})
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
    render_hidden_placeholder_scaled(hidden,total,style,1)
}

pub fn render_hidden_placeholder_scaled(
    hidden:usize,total:usize,style:&Options,pixel_scale:u32,
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
    render_notification_group_scaled(note,&prepared,0,pixel_scale)
}

/// Exact rectangle coordinates for the positive-margin subset of Mako's
/// notification stack. Mako's first top margin and each horizontal margin
/// are part of the surface geometry, not an outer layer-shell margin.
/// Negative overlapping margins still require compositor parity QA.
pub fn mako_stack_geometry(
    sizes:&[(u32,u32)],margins:&[crate::wayland::PopupMargins],
    alignment:StackAlignment,
)->Result<(u32,u32,Vec<(i32,i32)>),String>{
    if sizes.len()!=margins.len(){return Err("popup margin count mismatch".into());}
    if sizes.is_empty(){return Ok((0,0,Vec::new()));}
    // Mako v1.11 computes canvas width with signed side margins, even
    // when this clips some part of the notification at the surface edge.
    let width=sizes.iter().zip(margins).map(|((w,_),m)|
        i64::from(*w)+i64::from(m.left)+i64::from(m.right)
    ).max().unwrap_or(0);
    if !(1..=4096).contains(&width){return Err("invalid Mako popup width".into());}
    let mut end=0i64;
    let mut positions=Vec::with_capacity(sizes.len());
    for (index,((w,h),margin)) in sizes.iter().zip(margins).enumerate(){
        let space=if index==0{margin.top}
            else{margins[index-1].bottom.max(margin.top)};
        end=end.checked_add(i64::from(space)).ok_or("popup height overflow")?;
        let x=match alignment {
            StackAlignment::Left=>i64::from(margin.left),
            StackAlignment::Right=>width-i64::from(*w)-i64::from(margin.right),
            StackAlignment::Center=>(width-i64::from(*w))/2,
        };
        positions.push((
            i32::try_from(x).map_err(|_|"popup horizontal offset overflow")?,
            i32::try_from(end).map_err(|_|"popup vertical offset overflow")?,
        ));
        end=end.checked_add(i64::from(*h)).ok_or("popup height overflow")?;
    }
    // Pinned Mako v1.11.0 render.c returns total_height immediately after
    // drawing the last notification. It intentionally does NOT add that
    // final notification's pending bottom margin (whether positive or
    // negative). Newer Mako master later changed this, but strict MVP
    // must reproduce the pinned release, including this quirk.
    if !(1..=4096).contains(&end) || width*end*4>64*1024*1024 {
        return Err("popup stack exceeds safe Wayland bounds".into());
    }
    Ok((width as u32,end as u32,positions))
}

/// Composite one pre-multiplied Cairo ARGB32 pixel over its destination.
fn composite_bgra_over(destination:&mut [u8],source:&[u8]) {
    let alpha=u32::from(source[3]);
    if alpha==0{return;}
    if alpha==255{
        destination.copy_from_slice(source);
        return;
    }
    let inverse=255-alpha;
    for channel in 0..3 {
        destination[channel]=u8::try_from(
            (u32::from(source[channel])+
                (u32::from(destination[channel])*inverse+127)/255).min(255)
        ).unwrap();
    }
    destination[3]=u8::try_from(
        (alpha+(u32::from(destination[3])*inverse+127)/255).min(255)
    ).unwrap();
}

pub fn compose_mako_stack(
    frames:&[PixelFrame],margins:&[crate::wayland::PopupMargins],
    alignment:StackAlignment,
)->Result<Option<PixelFrame>,String>{
    compose_mako_stack_with_visible_count(frames,margins,alignment,frames.len())
}

/// Pinned Mako v1.11 render.c does not add its synthetic hidden-count popup
/// to max_width, even though that popup contributes height. Keep the normal
/// compositor unchanged; opt in only when the final frame is synthetic.
pub fn compose_mako_stack_with_visible_count(
    frames:&[PixelFrame],margins:&[crate::wayland::PopupMargins],
    alignment:StackAlignment,visible_count:usize,
)->Result<Option<PixelFrame>,String>{
    if visible_count>frames.len(){return Err("visible notification count exceeds stack".into());}
    if frames.is_empty(){
        if margins.is_empty(){return Ok(None);}
        return Err("popup margin count mismatch".into());
    }
    let sizes:Vec<_>=frames.iter().map(|f|(f.width,f.height)).collect();
    let (all_width,height,all_positions)=mako_stack_geometry(&sizes,margins,alignment)?;
    let width=if visible_count>0 && visible_count<frames.len(){
        mako_stack_geometry(&sizes[..visible_count],&margins[..visible_count],alignment)?.0
    }else{all_width};
    // Compute X against Mako's real-notification width, not against the
    // synthetic hidden popup's width. Y still includes that popup.
    let positions=all_positions.into_iter().zip(frames.iter().zip(margins))
        .map(|((_,y),(frame,margin))|{
            let x=match alignment{
                StackAlignment::Left=>i64::from(margin.left),
                StackAlignment::Right=>i64::from(width)-i64::from(frame.width)
                    -i64::from(margin.right),
                StackAlignment::Center=>(i64::from(width)-i64::from(frame.width))/2,
            };
            Ok((i32::try_from(x).map_err(|_|"popup horizontal offset overflow")?,y))
        }).collect::<Result<Vec<_>,String>>()?;
    let stride=width.checked_mul(4).ok_or("popup stride overflow")?;
    let size=u64::from(stride)*u64::from(height);
    let mut pixels=vec![0u8;size as usize];
    for (frame,(x,y)) in frames.iter().zip(positions){
        if frame.width==0||frame.height==0||
            frame.stride<i32::try_from(frame.width.saturating_mul(4))
                .map_err(|_|"invalid frame stride")?
            ||frame.pixels.len()!=frame.stride as usize*frame.height as usize{
            return Err("invalid child frame".into());
        }
        // Negative margins can place a notification outside the canvas.
        // Cairo clips those pixels; match this behavior with signed offsets
        // rather than translating or clamping the notification rectangle.
        let from_x=i64::from(x).max(0);
        let to_x=(i64::from(x)+i64::from(frame.width)).min(i64::from(width));
        let from_y=i64::from(y).max(0);
        let to_y=(i64::from(y)+i64::from(frame.height)).min(i64::from(height));
        if from_x>=to_x||from_y>=to_y{continue;}
        for py in from_y..to_y {
            let src_row=(py-i64::from(y)) as usize*frame.stride as usize;
            let dst_row=py as usize*stride as usize;
            for px in from_x..to_x{
                let src_offset=src_row+(px-i64::from(x)) as usize*4;
                let dst_offset=dst_row+px as usize*4;
                composite_bgra_over(
                    &mut pixels[dst_offset..dst_offset+4],
                    &frame.pixels[src_offset..src_offset+4],
                );
            }
        }
    }
    Ok(Some(PixelFrame{width,height,stride:stride as i32,pixels}))
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
    fn rounded_translucent_border_and_progress_match_pinned_mako_cairo_path(){
        // render.c:215-272 fills the radius-preserving centerline path,
        // clips progress to it, and finally strokes that SAME path with
        // SOURCE. A filled outer border plus inset background is not
        // equivalent at translucent antialiased rounded corners.
        let mut note=fixture();
        note.summary.clear();
        note.body.clear();
        note.progress=63;
        let style=crate::config::Config::parse(
            "width=95\nheight=90\npadding=0\nborder-size=4\n             border-radius=17\nformat=%s\n             background-color=#C22444B0\nborder-color=#1648AC85\n             progress-color=source #8B9237BF\n"
        ).unwrap().global;
        let frame=render_notification(&note,&style).unwrap();
        let reference=cairo::ImageSurface::create(cairo::Format::ARgb32,
            frame.width as i32,frame.height as i32).unwrap();
        let context=cairo::Context::new(&reference).unwrap();
        let b=4.0;
        let w=f64::from(frame.width);
        let h=f64::from(frame.height);
        let radius=CornerRadii{tl:17.0,tr:17.0,br:17.0,bl:17.0};
        rounded(&context,b/2.0,b/2.0,w-b,h-b,radius);
        set_color(&context,Color::parse_mako("#C22444B0").unwrap());
        context.fill_preserve().unwrap();
        let border_path=context.copy_path().unwrap();
        context.save().unwrap();
        context.clip();
        context.set_operator(cairo::Operator::Source);
        set_color(&context,Color::parse_mako("#8B9237BF").unwrap());
        let progress_width=((frame.width as i64-8)*63/100) as f64;
        context.rectangle(b,b,progress_width,h-b);
        context.fill().unwrap();
        context.restore().unwrap();
        context.new_path();
        context.append_path(&border_path);
        context.set_operator(cairo::Operator::Source);
        set_color(&context,Color::parse_mako("#1648AC85").unwrap());
        context.set_line_width(b);
        context.stroke().unwrap();
        reference.flush();
        let mut expected=Vec::new();
        reference.with_data(|pixels|expected.extend_from_slice(pixels)).unwrap();
        assert_eq!(frame.pixels,expected,
            "render.c's centerline SOURCE border and progress clipping differ");
        // An isolated straight border pixel should show premultiplied
        // translucent blue rather than opaque mixed background color.
        let y=(frame.height/2) as usize;
        let edge=y*frame.stride as usize+4;
        let blue=&frame.pixels[edge..edge+4];
        assert_eq!(blue[3],0x85,
            "SOURCE stroke must preserve border alpha instead of blending");
        for (channel,unpremultiplied) in [
            (blue[0],0xACi32),(blue[1],0x48i32),(blue[2],0x16i32)
        ]{
            let expected=(unpremultiplied*0x85+127)/255;
            assert!((i32::from(channel)-expected).abs()<=1,
                "premultiplied border channel {channel} differs from SOURCE {expected}");
        }
    }

    #[test]
    fn pinned_mako_signed_vertical_padding_reduces_shrinkwrapped_height(){
        // v1.11.0 render.c adds padding.top+padding.bottom directly
        // to the measured Pango text height, even when those are negative.
        // The previous unsigned clamp wrongly made both frames identical.
        let note=fixture();
        let common="width=180\nheight=150\nborder-size=0\nformat=%s\n";
        let plain=crate::config::Config::parse(
            &(common.to_owned()+"padding=0\n")
        ).unwrap().global;
        let negative=crate::config::Config::parse(
            &(common.to_owned()+"padding=-3,0,-2,0\n")
        ).unwrap().global;
        let pad=Padding::parse(&negative);
        assert_eq!((pad.top,pad.right,pad.bottom,pad.left),(-3,0,-2,0));
        let original=measure_notification_height(&note,&plain).unwrap();
        let shifted=measure_notification_height(&note,&negative).unwrap();
        assert_eq!(original,shifted+5,
            "Mako's signed -3 top and -2 bottom reduce content height by 5");
        let rendered=render_notification(&note,&negative).unwrap();
        assert_eq!(rendered.height,shifted,
            "the Cairo frame must preserve the signed Pango measurement");
    }

    #[test]
    fn signed_horizontal_padding_increases_mako_text_layout_width(){
        // v1.11.0 computes text_layout_width as width minus the signed
        // left and right padding. Negative sides give Pango MORE room
        // before text is clipped at the actual popup surface edge.
        let mut note=fixture();
        note.summary="ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZ".into();
        let mut zero=Options::new();
        for (key,value) in [
            ("width","84"),("height","250"),("border-size","0"),
            ("font","monospace 16"),("format","%s"),("padding","0"),
        ]{zero.insert(key.into(),value.into());}
        let mut negative=zero.clone();
        negative.insert("padding".into(),"0,-16,0,-16".into());
        let pad=Padding::parse(&negative);
        assert_eq!((pad.left,pad.right),(-16,-16));
        let original=measure_notification_height(&note,&zero).unwrap();
        let expanded=measure_notification_height(&note,&negative).unwrap();
        assert!(expanded<original,
            "negative horizontal insets increase wrap width and reduce text lines");
        assert_eq!(render_notification(&note,&negative).unwrap().height,expanded);
    }

    #[test]
    fn large_corner_radius_can_extend_beyond_mako_text_height_cap() {
        let note=fixture();
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","35"),("border-size","2"),
            ("border-radius","30,5,30,5"),
        ]{style.insert(key.into(),value.into());}
        // The top-left/bottom-right corners are each 30, so neither
        // side needs 60px. The opposite same-side sum is 35px.
        let height=measure_notification_height(&note,&style).unwrap();
        assert!(height>=39);
        style.insert("border-radius".into(),"40,5,5,40".into());
        assert!(measure_notification_height(&note,&style).unwrap()>=84);
        assert!(render_notification(&note,&style).unwrap().height>=84);
    }

    #[test]
    fn source_scoped_radius_padding_resolution_reaches_renderer() {
        let fields=crate::config::Traits {
            app_name:"fixture",app_icon:"",category:"success",
            desktop_entry:"",summary:"Text visible",body:"Line 2",
            urgency:"normal",anchor:"top-right",output:"",
            surface_associated:true,output_known:true,modes:&["default"],requested_timeout_ms:-1,actionable:false,
            group_index:-1,hidden:false,
        };
        let config=crate::config::Config::parse(
            "border-radius=24\n[category=success]\npadding=3\n"
        ).unwrap();
        let styled=config.style_for(&fields);
        assert_eq!(styled["padding"],"3",
            "scoped padding does not inherit a new root-style clamp");
        assert_eq!(Padding::parse(&styled).left,3);
        let config=crate::config::Config::parse(
            "[category=success]\npadding=3\nborder-radius=24\n"
        ).unwrap();
        let styled=config.style_for(&fields);
        let pad=Padding::parse(&styled);
        assert_eq!((pad.top,pad.right,pad.bottom,pad.left),(3,24,3,24));
        let note=fixture();
        assert_eq!(render_notification(&note,&styled).unwrap().height,
            measure_notification_height(&note,&styled).unwrap(),
            "renderer measurement and paint must share effective padding");
    }

    #[test]
    fn legacy_strtoul_color_forms_render_their_exact_background_pixels() {
        for (value,expected_bgra) in [
            ("#+fffff",[0xff,0xff,0x0f,0xff]),
            ("#0x1234",[0x34,0x12,0x00,0xff]),
            ("#-fffff",[0x01,0x00,0xf0,0xff]),
        ] {
            let config=crate::config::Config::parse(&format!(
                "background-color={value}\nwidth=180\nheight=120\nborder-size=0\n"
            )).unwrap();
            let frame=render_notification(&fixture(),&config.global).unwrap();
            let x=(frame.width as usize)/2;
            let y=(frame.height as usize).saturating_sub(2);
            let offset=y*frame.stride as usize+x*4;
            assert_eq!(&frame.pixels[offset..offset+4],&expected_bgra,
                "incorrect Cairo background for {value}");
        }
    }

    #[test]
    fn directional_padding_affects_actual_notification_height() {
        let mut style=Options::new();
        for (key,value) in [("width","240"),("height","180"),
            ("border-size","0"),("padding","2,4,6,8")] {
            style.insert(key.into(),value.into());
        }
        let pad=Padding::parse(&style);
        assert_eq!((pad.top,pad.right,pad.bottom,pad.left),(2,4,6,8));
        let original=measure_notification_height(&fixture(),&style).unwrap();
        style.insert("padding".into(),"20,4,30,8".into());
        let taller=measure_notification_height(&fixture(),&style).unwrap();
        assert!(taller>original,"vertical padding changes popup height");
        let frame=render_notification(&fixture(),&style).unwrap();
        assert_eq!(frame.height,taller);
    }

    #[test]
    fn mako_directional_border_radii_draw_each_corner_independently() {
        let note=fixture();
        let mut style=Options::new();
        for (key,value) in [
            ("width","120"),("height","100"),("border-size","0"),
            ("border-radius","24,0,0,0"),
            ("background-color","#305747"),
        ]{style.insert(key.into(),value.into());}
        assert_eq!(CornerRadii::parse(&style),CornerRadii{
            tl:24.0,tr:0.0,br:0.0,bl:0.0
        });
        let frame=render_notification(&note,&style).unwrap();
        let pixel=|x:usize,y:usize|{
            let i=y*frame.stride as usize+x*4;
            [frame.pixels[i],frame.pixels[i+1],frame.pixels[i+2],frame.pixels[i+3]]
        };
        assert_eq!(pixel(0,0)[3],0,"rounded top-left corner transparent");
        assert_eq!(pixel(frame.width as usize-1,0)[3],255,
            "square top-right corner remains filled");
        assert_eq!(pixel(0,frame.height as usize-1)[3],255,
            "square bottom-left corner remains filled");
        style.insert("border-radius".into(),"4,8,12,16".into());
        assert_eq!(CornerRadii::parse(&style),CornerRadii{
            tl:4.0,tr:8.0,br:12.0,bl:16.0
        });
        assert!(CornerRadii::parse(&style).required_height(2)>=24);
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
    fn icon_border_radius_clips_image_without_erasing_background() {
        let mut note=fixture();
        note.image_data=crate::icons::decode_raw_image(
            12,12,48,true,8,4,&[255,0,0,255].repeat(144));
        let mut style=Options::new();
        for (key,value) in [
            ("width","180"),("height","90"),("border-size","0"),
            ("padding","4"),("icon-border-radius","6"),
            ("background-color","#305747"),
        ]{style.insert(key.into(),value.into());}
        let rounded=render_notification(&note,&style).unwrap();
        style.insert("icon-border-radius".into(),"0".into());
        let square=render_notification(&note,&style).unwrap();
        let icon_center_y=rounded.height as usize/2;
        let icon_top=icon_center_y-6;
        let corner=icon_top*rounded.stride as usize+4*4;
        assert_eq!(&square.pixels[corner..corner+4],&[0,0,255,255]);
        assert_ne!(&rounded.pixels[corner..corner+4],&[0,0,255,255]);
        let interior=icon_center_y*rounded.stride as usize+10*4;
        assert_eq!(&rounded.pixels[interior..interior+4],&[0,0,255,255]);
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
    fn pinned_mako_icon_text_offsets_and_axis_centering(){
        // Pinned render.c: left text_x=icon.width+2*padding.left;
        // top text_y=icon.height+2*padding.top. For horizontal icons it
        // centers a short Pango line alongside a tall image; for vertical
        // icons it centers the line across the notification width.
        let mut note=fixture();
        note.summary="I".into();
        note.body.clear();
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","140"),("padding","4"),
            ("border-size","0"),("font","monospace 14"),
            ("format","%s"),("text-color","#FFFFFFFF"),
            ("background-color","#000000FF"),("icon-location","left"),
            ("max-icon-size","48"),
        ]{style.insert(key.into(),value.into());}
        let bounds=|frame:&PixelFrame|{
            let (mut x,mut y)=(frame.width,frame.height);
            for yy in 0..frame.height as usize{
                for xx in 0..frame.width as usize{
                    let i=yy*frame.stride as usize+xx*4;
                    let pixel=&frame.pixels[i..i+4];
                    if pixel[0]>180 && pixel[1]>180 && pixel[2]>180 && pixel[3]>220 {
                        x=x.min(xx as u32);
                        y=y.min(yy as u32);
                    }
                }
            }
            assert!(x<frame.width && y<frame.height,"expected actual white Pango glyph pixels");
            (x,y)
        };
        let bare=bounds(&render_notification(&note,&style).unwrap());
        note.image_data=crate::icons::decode_raw_image(
            12,28,48,true,8,4,&[255,0,0,255].repeat(12*28));
        let left=bounds(&render_notification(&note,&style).unwrap());
        assert_eq!(left.0,bare.0+16,
            "left-icon offset must equal icon width + one additional left padding");
        assert!(left.1>bare.1,
            "Mako centers short text vertically beside a taller icon");

        note.image_data=crate::icons::decode_raw_image(
            12,12,48,true,8,4,&[255,0,0,255].repeat(12*12));
        style.insert("icon-location".into(),"top".into());
        let top=bounds(&render_notification(&note,&style).unwrap());
        let mut no_icon=note.clone();
        no_icon.image_data=None;
        let vertical_bare=bounds(&render_notification(&no_icon,&style).unwrap());
        assert_eq!(top.1,vertical_bare.1+16,
            "top-icon offset must equal icon height + one additional top padding");
        assert!(top.0>bare.0+30,
            "Mako centers text horizontally below a top icon");
    }

    #[test]
    fn fractional_icon_scaling_truncates_dimensions_without_second_shrink(){
        // Mako v1.11.0 icon.c uses (int)(source_dimension * scale).
        // A 12x7 image at max-icon-size=8 occupies an 8x4 logical clip,
        // NOT 8x5. The full 8-pixel width must still be painted when the
        // measured four-pixel popup cannot contain all 4.67 source pixels.
        let mut note=fixture();
        note.summary.clear();
        note.body.clear();
        note.image_data=crate::icons::decode_raw_image(
            12,7,48,true,8,4,&[255,0,0,255].repeat(12*7));
        let mut style=Options::new();
        for (key,value) in [
            ("width","100"),("height","50"),("border-size","0"),
            ("padding","0"),("format","%s"),("max-icon-size","8"),
            ("icon-location","left"),("background-color","#000000FF"),
        ] {style.insert(key.into(),value.into());}
        assert_eq!(measure_notification_height(&note,&style).unwrap(),4,
            "fractional icon logical height must truncate, not round");
        let frame=render_notification(&note,&style).unwrap();
        assert_eq!((frame.width,frame.height),(100,4));
        let at=|x:usize,y:usize|{
            let p=y*frame.stride as usize+x*4;
            frame.pixels[p..p+4].to_vec()
        };
        assert_eq!(&at(7,3)[..2],&[0,0]);
        assert!(at(7,3)[2]>=254 && at(7,3)[3]==255,
            "renderer must retain all eight Mako icon pixels");
        assert_eq!(at(8,3),vec![0,0,0,255],
            "fractional image must be clipped at integer icon width");
    }

    #[test]
    fn vertical_icons_ignore_asymmetric_text_padding_when_centering(){
        // Pinned render.c centers vertical icons using
        // (notif_width - icon.width - border_size)/2, independent of
        // the left/right text padding.
        let mut note=fixture();
        note.summary.clear();
        note.body.clear();
        note.image_data=crate::icons::decode_raw_image(
            12,7,48,true,8,4,&[255,0,0,255].repeat(12*7));
        let mut style=Options::new();
        for (key,value) in [
            ("width","100"),("height","50"),("border-size","0"),
            ("padding","5,8,5,2"),("format","%s"),("max-icon-size","8"),
            ("icon-location","top"),("background-color","#000000FF"),
        ] {style.insert(key.into(),value.into());}
        let frame=render_notification(&note,&style).unwrap();
        let at=|x:usize,y:usize|{
            let p=y*frame.stride as usize+x*4;
            frame.pixels[p..p+4].to_vec()
        };
        assert_eq!(at(45,5),vec![0,0,0,255]);
        for x in [46,53] {
            let pixel=at(x,5);
            assert_eq!(&pixel[..2],&[0,0]);
            assert!(pixel[2]>=254 && pixel[3]==255,
                "the integer icon clip must include its boundary column");
        }
        assert_eq!(at(54,5),vec![0,0,0,255]);
    }

    #[test]
    fn file_url_relative_icon_path_renders_sender_provided_image() {
        // CI runs this in a scratch test checkout. Materializing the
        // fixture below cwd avoids mutating any user's Mako icon theme.
        let cwd=std::env::current_dir().unwrap();
        let dir=tempfile::tempdir_in(&cwd).unwrap();
        let file=dir.path().join("relative-fixture.png");
        let pixbuf=gdk_pixbuf::Pixbuf::new(
            gdk_pixbuf::Colorspace::Rgb,true,8,12,12
        ).unwrap();
        pixbuf.fill(0xFF0000FF);
        pixbuf.savev(&file,"png",&[]).unwrap();
        let relative=file.strip_prefix(&cwd).unwrap();
        let mut note=fixture();
        note.app_icon=format!("file://{}",relative.display());
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","70"),("padding","4"),
            ("border-size","0"),("icon-location","left"),
            ("max-icon-size","16"),("background-color","#305747")
        ] {style.insert(key.into(),value.into());}
        let frame=render_notification(&note,&style).unwrap();
        let at=(frame.height as usize/2)*frame.stride as usize+6*4;
        assert_eq!(&frame.pixels[at..at+4],&[0,0,255,255],
            "relative file:// URLs must render actual pixels like Mako");
    }

    #[test]
    fn file_url_with_non_utf8_linux_filename_renders_image() {
        use std::os::unix::ffi::OsStringExt;
        let dir=tempfile::tempdir().unwrap();
        let source=dir.path().join("source.png");
        let mut non_utf8=dir.path().to_path_buf();
        non_utf8.push(std::ffi::OsString::from_vec(b"icon\xff.png".to_vec()));
        let pixbuf=gdk_pixbuf::Pixbuf::new(
            gdk_pixbuf::Colorspace::Rgb,true,8,12,12
        ).unwrap();
        pixbuf.fill(0xFF0000FF);
        pixbuf.savev(&source,"png",&[]).unwrap();
        std::fs::rename(&source,&non_utf8).unwrap();
        let mut note=fixture();
        note.app_icon=format!("file://{}/icon%FF.png",dir.path().display());
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","70"),("padding","4"),
            ("border-size","0"),("icon-location","left"),
            ("max-icon-size","16"),("background-color","#305747")
        ] {style.insert(key.into(),value.into());}
        let frame=render_notification(&note,&style).unwrap();
        let at=(frame.height as usize/2)*frame.stride as usize+6*4;
        assert_eq!(&frame.pixels[at..at+4],&[0,0,255,255],
            "Mako file URL must load Linux filename bytes outside UTF-8");
    }

    #[test]
    fn icon_lookup_uses_maximum_monitor_scale_without_scaling_popup(){
        let dir=tempfile::tempdir().unwrap();
        for (name,rgba) in [
            ("16x16/apps",0xFF0000FF),
            ("16x16@2/apps",0x0000FFFF),
        ] {
            let path=dir.path().join(name).join("posthorn.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let pixbuf=gdk_pixbuf::Pixbuf::new(
                gdk_pixbuf::Colorspace::Rgb,true,8,12,12
            ).unwrap();
            pixbuf.fill(rgba);
            pixbuf.savev(&path,"png",&[]).unwrap();
        }
        let mut note=fixture();
        note.app_icon="posthorn".into();
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","70"),("padding","4"),
            ("border-size","0"),("icon-location","left"),
            ("max-icon-size","16"),("background-color","#305747"),
        ] {style.insert(key.into(),value.into());}
        style.insert("icon-path".into(),dir.path().to_string_lossy().into_owned());
        let native=render_notification_group_with_icon_scale(&note,&style,0,1,1).unwrap();
        let with_hidpi_monitor=
            render_notification_group_with_icon_scale(&note,&style,0,1,2).unwrap();
        assert_eq!((native.width,native.height,native.stride),
            (with_hidpi_monitor.width,with_hidpi_monitor.height,
                with_hidpi_monitor.stride),
            "the other monitor must not enlarge this 1x popup");
        assert_ne!(native.pixels,with_hidpi_monitor.pixels,
            "max output scale must select a distinct HiDPI theme asset");
    }

    #[test]
    fn actual_icon_pixels_follow_mako_relative_path_scale(){
        let dir=tempfile::tempdir().unwrap();
        for (folder,rgba) in [
            ("16x16/apps",0xFF0000FF),
            ("16x16/apps@2",0x00FF00FF),
        ] {
            let path=dir.path().join(folder).join("posthorn.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let pixbuf=gdk_pixbuf::Pixbuf::new(
                gdk_pixbuf::Colorspace::Rgb,true,8,12,12
            ).unwrap();
            pixbuf.fill(rgba);
            pixbuf.savev(&path,"png",&[]).unwrap();
        }
        let mut note=fixture();
        note.app_icon="posthorn".into();
        let mut style=Options::new();
        for (key,value) in [
            ("width","160"),("height","70"),("padding","4"),
            ("border-size","0"),("icon-location","left"),
            ("max-icon-size","16"),("background-color","#305747"),
        ] {style.insert(key.into(),value.into());}
        style.insert("icon-path".into(),dir.path().to_string_lossy().into_owned());
        let native=render_notification_group_with_icon_scale(&note,&style,0,1,1).unwrap();
        let hidpi=render_notification_group_with_icon_scale(&note,&style,0,1,2).unwrap();
        assert_eq!((native.width,native.height),(hidpi.width,hidpi.height),
            "the selected scale must not enlarge the popup");
        let index=|frame:&PixelFrame|{
            (frame.height as usize/2)*frame.stride as usize+6*4
        };
        assert_eq!(&native.pixels[index(&native)..index(&native)+4],
            &[0,0,255,255],"Mako selects the unscaled category asset at 1x");
        assert_eq!(&hidpi.pixels[index(&hidpi)..index(&hidpi)+4],
            &[0,255,0,255],"Mako reads @2 from a later category at 2x");
    }

    #[test]
    fn progress_fill_uses_integer_logical_pixel_width_before_hidpi_scaling(){
        let mut style=Options::new();
        for (key,value) in [
            ("width","101"),("height","70"),("padding","0"),
            ("border-size","0"),("background-color","#0000FFFF"),
            ("progress-color","source #FF0000FF"),
        ] {style.insert(key.into(),value.into());}
        let mut note=fixture();
        for border in [0usize,2] {
            style.insert("border-size".into(),border.to_string());
            let inner_width=101-2*border;
            for value in [1,33,49,66,99,100] {
                note.progress=value;
                for scale in [1u32,2] {
                    let factor=scale as usize;
                    let frame=render_notification_group_scaled(&note,&style,0,scale).unwrap();
                    let logical_filled=inner_width*(value as usize)/100;
                    let y=(frame.height as usize).saturating_sub((border+3)*factor);
                    let color_at=|physical_x:usize| {
                        let start=y*(frame.stride as usize)+physical_x*4;
                        frame.pixels[start..start+4].to_vec()
                    };
                    if logical_filled>0 {
                        assert_eq!(color_at((border+logical_filled)*factor-1),
                            [0,0,255,255],
                            "last fill pixel not solid at {value}% border={border} {scale}x");
                    }
                    if logical_filled<inner_width {
                        assert_eq!(color_at((border+logical_filled)*factor),
                            [255,0,0,255],
                            "unexpected antialiasing at {value}% border={border} {scale}x");
                    }
                }
            }
        }
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
        note.progress=-9;
        let negative=render_notification(&note,&style).unwrap();
        assert_eq!(negative.pixels,empty.pixels,
            "Mako clamps negative progress to zero width");
        note.progress=100;
        let full=render_notification(&note,&style).unwrap();
        note.progress=170;
        let too_large=render_notification(&note,&style).unwrap();
        assert_eq!(too_large.pixels,full.pixels,
            "Mako clamps excessive progress width at 100%");
    }

    #[test]
    fn progress_source_blend_uses_mako_color_even_with_ignored_suffix(){
        let mut note=fixture();
        note.progress=100;
        let mut style=Options::new();
        for (key,value) in [
            ("width","120"),("height","90"),("border-size","0"),
            ("background-color","#000000"),
            ("progress-color","SoUrCe #FF000080 extra"),
        ]{style.insert(key.into(),value.into());}
        let source=render_notification(&note,&style).unwrap();
        let at=(source.height as usize-3)*source.stride as usize+10*4;
        assert_eq!(&source.pixels[at..at+4],&[0,0,128,128],
            "SOURCE must replace alpha, not blend over opaque background");
        style.insert("progress-color".into(),"OVER #FF000080 ignored".into());
        let over=render_notification(&note,&style).unwrap();
        assert_eq!(&over.pixels[at..at+4],&[0,0,128,255],
            "OVER must retain the existing opaque background alpha");
    }

    #[test]
    fn c_strtoul_color_variants_reach_configured_progress_source_pixels() {
        let config=crate::config::Config::parse(
            "background-color=#000000\nprogress-color=source #+fffff\n             border-size=0\nwidth=120\nheight=90\n"
        ).unwrap();
        let mut note=fixture();
        note.progress=100;
        let frame=render_notification(&note,&config.global).unwrap();
        let offset=(frame.height as usize-3)*frame.stride as usize+10*4;
        assert_eq!(&frame.pixels[offset..offset+4],&[255,255,15,255],
            "Mako source #+fffff must paint the parsed 0x0fffffff value");
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
    fn pinned_mako_v1_11_omits_final_bottom_margin_even_when_negative() {
        use crate::wayland::PopupMargins;
        let margins=[PopupMargins{top:2,right:0,bottom:5,left:0}];
        let (width,height,coords)=mako_stack_geometry(
            &[(12,10)],&margins,StackAlignment::Left
        ).unwrap();
        assert_eq!((width,height),(12,12));
        assert_eq!(coords,vec![(0,2)]);
        let negative=[PopupMargins{bottom:-3,..margins[0]}];
        assert_eq!(mako_stack_geometry(
            &[(12,10)],&negative,StackAlignment::Left
        ).unwrap().1,12);
    }

    #[test]
    fn negative_margins_overlap_and_clip_like_upstream_mako() {
        use crate::wayland::PopupMargins;
        let red=PixelFrame{
            width:4,height:3,stride:16,pixels:[0u8,0,255,255].repeat(12),
        };
        let blue=PixelFrame{
            width:4,height:3,stride:16,pixels:[255u8,0,0,255].repeat(12),
        };
        let margins=[
            PopupMargins{top:-1,right:-1,bottom:-2,left:-1},
            PopupMargins{top:-1,right:-1,bottom:0,left:-1},
        ];
        let (w,h,positions)=mako_stack_geometry(
            &[(4,3),(4,3)],&margins,StackAlignment::Left
        ).unwrap();
        assert_eq!((w,h),(2,4));
        assert_eq!(positions,vec![(-1,-1),(-1,1)]);
        let image=compose_mako_stack(&[red,blue],&margins,StackAlignment::Left)
            .unwrap().unwrap();
        assert_eq!((image.width,image.height),(2,4));
        assert_eq!(&image.pixels[0..4],&[0,0,255,255],
            "the second popup begins on row 1, leaving row 0 red");
        assert_eq!(&image.pixels[image.stride as usize..image.stride as usize+4],
            &[255,0,0,255],"the later drawn popup covers the overlap");
        assert_eq!(&image.pixels[image.stride as usize*3..image.stride as usize*3+4],
            &[255,0,0,255]);
    }

    #[test]
    fn overlapping_argb_pixels_use_source_over_not_blind_replacement() {
        let mut destination=[0u8,0,255,255];
        composite_bgra_over(&mut destination,&[0,0,0,0]);
        assert_eq!(destination,[0,0,255,255]);
        composite_bgra_over(&mut destination,&[128,0,0,128]);
        assert_eq!(destination,[128,0,127,255]);
    }

    #[test]
    fn actual_mako_surface_geometry_includes_first_top_and_side_margins(){
        let m=[
            crate::wayland::PopupMargins{top:5,right:7,bottom:11,left:13},
            crate::wayland::PopupMargins{top:4,right:3,bottom:2,left:6},
        ];
        let (w,h,left)=mako_stack_geometry(&[(100,20),(80,10)],&m,StackAlignment::Left).unwrap();
        assert_eq!((w,h),(120,46));
        assert_eq!(left,vec![(13,5),(6,36)]);
        let (_,_,right)=mako_stack_geometry(&[(100,20),(80,10)],&m,StackAlignment::Right).unwrap();
        assert_eq!(right,vec![(13,5),(37,36)]);
        let frame=|width,height,value|PixelFrame{
            width,height,stride:(width*4) as i32,
            pixels:vec![value;(width*height*4) as usize],
        };
        let composed=compose_mako_stack(&[frame(100,20,9),frame(80,10,7)],&m,
            StackAlignment::Left).unwrap().unwrap();
        assert_eq!((composed.width,composed.height),(120,46));
        assert!(composed.pixels[0..(5*120*4) as usize].iter().all(|v|*v==0));
        assert_eq!(composed.pixels[(5*120+13)*4],9);
        assert_eq!(composed.pixels[(36*120+6)*4],7);
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
    fn mako_hidden_synthetic_frame_cannot_expand_real_canvas(){
        let solid=|width:u32,byte:u8|PixelFrame{
            width,height:2,stride:(width*4) as i32,
            pixels:(0..width*2).flat_map(|_|[byte,byte,byte,255]).collect(),
        };
        let visible=solid(6,40);
        let placeholder=solid(16,90);
        let margin=crate::wayland::PopupMargins::uniform(1);
        let frames=[visible.clone(),placeholder];
        let limits=[margin,margin];
        let ordinary=compose_mako_stack(&frames,&limits,StackAlignment::Right)
            .unwrap().unwrap();
        assert_eq!(ordinary.width,18);
        let clipped=compose_mako_stack_with_visible_count(
            &frames,&limits,StackAlignment::Right,1
        ).unwrap().unwrap();
        assert_eq!((clipped.width,clipped.height),(8,6));
        let pixel=|x:usize,y:usize|{
            let off=y*clipped.stride as usize+x*4;
            &clipped.pixels[off..off+4]
        };
        assert_eq!(pixel(0,1),&[0,0,0,0]);
        assert_eq!(pixel(1,1),&[40,40,40,255]);
        assert_eq!(pixel(6,1),&[40,40,40,255]);
        assert_eq!(pixel(7,1),&[0,0,0,0]);
        assert!(compose_mako_stack_with_visible_count(
            &frames,&limits,StackAlignment::Right,3
        ).is_err());
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
    fn native_scale_draws_pango_at_physical_resolution() {
        let note=fixture();
        let style=crate::config::Config::default().global;
        let logical=render_notification_group_scaled(&note,&style,0,1).unwrap();
        let native=render_notification_group_scaled(&note,&style,0,2).unwrap();
        assert_eq!(native.width,logical.width*2);
        assert_eq!(native.height,logical.height*2);
        assert_eq!(native.stride,native.width as i32*4);
        let stretched=crate::wayland::scale_frame(&logical,2).unwrap();
        // Glyph edges rendered directly at 2x must not be pixel copies of
        // the existing 1x antialiasing pattern.
        assert_ne!(native.pixels,stretched.pixels);
        assert!(render_notification_group_scaled(&note,&style,0,0).is_err());
        let scale_nine=render_notification_group_scaled(&note,&style,0,9).unwrap();
        assert_eq!(scale_nine.width,logical.width*9);
        assert_eq!(scale_nine.height,logical.height*9);
        assert!(render_notification_group_scaled(&note,&style,0,4096).is_err());
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
    fn large_radius_path_matches_unclamped_mako_cairo_arcs(){
        let draw=|ours:bool|{
            let mut image=cairo::ImageSurface::create(cairo::Format::ARgb32,80,80).unwrap();
            let cr=cairo::Context::new(&image).unwrap();
            let radii=CornerRadii{tl:30.0,tr:12.0,br:25.0,bl:10.0};
            if ours {
                rounded(&cr,5.0,7.0,40.0,40.0,radii);
            }else{
                // Upstream Mako render.c:set_rounded_rectangle(), applied
                // directly with no half-width/height radius cap.
                use std::f64::consts::{PI,FRAC_PI_2};
                cr.new_sub_path();
                cr.arc(5.0+30.0,7.0+30.0,30.0,PI,3.0*FRAC_PI_2);
                cr.arc(5.0+40.0-12.0,7.0+12.0,12.0,-FRAC_PI_2,0.0);
                cr.arc(5.0+40.0-25.0,7.0+40.0-25.0,25.0,0.0,FRAC_PI_2);
                cr.arc(5.0+10.0,7.0+40.0-10.0,10.0,FRAC_PI_2,PI);
                cr.close_path();
            }
            cr.set_source_rgb(1.0,1.0,1.0);
            cr.fill().unwrap();
            drop(cr);
            image.flush();
            let bytes=image.data().unwrap().to_vec();
            bytes
        };
        assert_eq!(draw(true),draw(false),
            "Mako never caps each corner radius at half the rectangle");
    }

    #[test]
    fn malformed_final_markup_preserves_formatted_template_as_plain_text(){
        let dummy=cairo::ImageSurface::create(cairo::Format::ARgb32,1,1).unwrap();
        let cr=cairo::Context::new(&dummy).unwrap();
        let layout=pangocairo::functions::create_layout(&cr);
        set_mako_formatted_text(&layout,"<b>Alert: First</b>");
        assert_eq!(layout.text().as_str(),"Alert: First");
        set_mako_formatted_text(&layout,"<b>Alert: First");
        assert_eq!(layout.text().as_str(),"<b>Alert: First",
            "Mako leaves the final malformed template visible literally");

        let mut note=fixture();
        note.summary="First".into();
        note.body="Second".into();
        let mut style=Options::new();
        style.insert("format".into(),"<b>Alert: %s".into());
        let decoded=decode_format(&style["format"]).unwrap();
        let text=render_notification_format(&decoded,&FormatNotification{
            id:note.id,app_name:&note.app_name,summary:&note.summary,
            body:&note.body,group_count:0,
        },false);
        assert_eq!(text,"<b>Alert: First");
        set_mako_formatted_text(&layout,&text);
        assert_eq!(layout.text().as_str(),text);
        assert!(measure_notification_height(&note,&style).is_ok());
        assert!(render_notification(&note,&style).is_ok());
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
    fn compositor_scale_above_eight_uses_checked_frame_bounds() {
        // Mako accepts wl_output.scale 9. Render within the same
        // conservative pixel/SHM caps used for any other scale.
        let note=fixture();
        let mut style=Options::new();
        style.insert("width".into(),"80".into());
        style.insert("height".into(),"60".into());
        style.insert("padding".into(),"0".into());
        let nine=render_notification_group_scaled(&note,&style,0,9).unwrap();
        assert_eq!(nine.width,720);
        assert_eq!(nine.height%9,0);
        assert!(render_notification_group_scaled(&note,&style,0,0).is_err());
        assert!(render_notification_group_scaled(&note,&style,0,4096).is_err());
    }

    #[test]
    fn absurd_surfaces_rejected_before_allocating() {
        let mut style = Options::new();
        style.insert("width".into(), "50000".into());
        assert!(render_notification(&fixture(), &style).is_err());
    }
}
