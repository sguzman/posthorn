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

pub fn render_notification(note: &Notification, style: &Options) -> Result<PixelFrame, String> {
    let width = integer_option(style, "width", 300);
    let height = integer_option(style, "height", 100);
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

    let markup = style.get("markup")
        .is_none_or(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let body_is_valid_markup = markup && pango::parse_markup(&note.body, '\0').is_ok();
    let format = style.get("format").map(String::as_str).unwrap_or("<b>%s</b>\\n%b");
    let decoded = decode_format(format).map_err(|e| e.to_string())?;
    let rendered = render_notification_format(&decoded, &FormatNotification {
        id: note.id, app_name: &note.app_name, summary: &note.summary,
        body: &note.body, group_count: 0,
    }, body_is_valid_markup);
    // If the configured template is malformed, escape *everything* instead of
    // interpreting unintended markup from a remote notification sender.
    let safe_markup = if pango::parse_markup(&rendered, '\0').is_ok() {
        rendered
    } else {
        escape_markup(&format!("{}\\n{}", note.summary, note.body))
    };

    let available_w = (f64::from(width) - (border + padding)*2.0).max(1.0);
    let available_h = (f64::from(height) - (border + padding)*2.0).max(1.0);
    context.rectangle(border+padding, border+padding, available_w, available_h);
    context.clip();
    context.move_to(border+padding, border+padding);
    set_color(&context, foreground);
    let layout = pangocairo::functions::create_layout(&context);
    let font = style.get("font").map(String::as_str).unwrap_or("monospace 10");
    layout.set_font_description(Some(&pango::FontDescription::from_string(font)));
    layout.set_width((available_w as i32).saturating_mul(pango::SCALE));
    layout.set_wrap(pango::WrapMode::WordChar);
    layout.set_markup(&safe_markup);
    pangocairo::functions::show_layout(&context, &layout);
    drop(layout);
    drop(context);
    surface.flush();
    let stride = surface.stride();
    let pixels = surface.with_data(|data| data.to_vec()).map_err(|e| e.to_string())?;
    Ok(PixelFrame {width, height, stride, pixels})
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
    fn produces_a_pango_raster_with_colored_background() {
        let mut style = Options::new();
        style.insert("width".into(), "280".into());
        style.insert("height".into(), "110".into());
        style.insert("border-size".into(), "2".into());
        style.insert("border-color".into(), "#FFFFFF".into());
        style.insert("background-color".into(), "#305747".into());
        style.insert("font".into(), "Sans 13".into());
        let pixels = render_notification(&fixture(), &style).unwrap();
        assert_eq!((pixels.width,pixels.height), (280,110));
        assert_eq!(pixels.stride,1120);
        let bg = ((90usize*pixels.stride as usize) + 100*4) as usize;
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
