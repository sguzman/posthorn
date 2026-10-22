//! CPU-only, premultiplied ARGB8888 pixel generation for Wayland SHM.
//! This is a building block for Mako's Cairo/Pango rendering parity, not yet
//! a replacement for Mako's text/icon rendering.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Color { pub red: u8, pub green: u8, pub blue: u8, pub alpha: u8 }

impl Color {
    /// Accept Mako's #RRGGBB and #RRGGBBAA forms.
    pub fn parse_mako(input: &str) -> Result<Self, &'static str> {
        let hex = input.strip_prefix('#').ok_or("expected a # prefix")?;
        if hex.len() != 6 && hex.len() != 8 {
            return Err("expected six or eight hexadecimal digits");
        }
        let component = |range: std::ops::Range<usize>|
            u8::from_str_radix(&hex[range], 16).map_err(|_| "invalid hex digit");
        Ok(Self {
            red: component(0..2)?,
            green: component(2..4)?,
            blue: component(4..6)?,
            alpha: if hex.len() == 8 { component(6..8)? } else { 255 },
        })
    }

    pub fn argb8888_le(self) -> [u8; 4] {
        let premul = |component: u8| -> u8 {
            ((u16::from(component) * u16::from(self.alpha) + 127) / 255) as u8
        };
        [premul(self.blue), premul(self.green), premul(self.red), self.alpha]
    }
}

/// The only Cairo operators accepted by Mako's progress-color parser.
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub enum ProgressBlend { Over, Source }

/// Mirrors Mako 1.11 parse_mako_color(): tokenize on ASCII space/tab only,
/// accept case-insensitive "over" and "source", and ignore later tokens.
pub fn parse_progress_color_mako(raw:&str)->Result<(ProgressBlend,Color),&'static str>{
    let mut tokens=raw.split([' ','\t']).filter(|token|!token.is_empty());
    let first=tokens.next().ok_or("missing progress color")?;
    let (blend,color)=if first.starts_with('#'){
        (ProgressBlend::Over,first)
    }else if first.eq_ignore_ascii_case("over"){
        (ProgressBlend::Over,tokens.next().ok_or("missing progress color")?)
    }else if first.eq_ignore_ascii_case("source"){
        (ProgressBlend::Source,tokens.next().ok_or("missing progress color")?)
    }else{
        return Err("invalid progress color operator");
    };
    Ok((blend,Color::parse_mako(color)?))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RectangleStyle {
    pub background: Color,
    pub border: Color,
    pub border_size: u32,
    pub border_radius: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PaintError { ZeroDimension, BufferTooLarge }

impl std::fmt::Display for PaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PaintError {}

fn inside_rounded(x: u32, y: u32, width: u32, height: u32, radius: u32) -> bool {
    if width == 0 || height == 0 { return false; }
    let r = radius.min(width / 2).min(height / 2) as f64;
    if r == 0.0 { return true; }
    let px = f64::from(x) + 0.5;
    let py = f64::from(y) + 0.5;
    let cx = px.clamp(r, f64::from(width) - r);
    let cy = py.clamp(r, f64::from(height) - r);
    (px - cx).powi(2) + (py - cy).powi(2) <= r * r
}

/// Produce a SHM-compatible byte array in BGRA order (little endian ARGB8888).
/// Pixels outside rounded corners have zero alpha.
pub fn paint_rounded(width: u32, height: u32, style: RectangleStyle)
    -> Result<Vec<u8>, PaintError>
{
    if width == 0 || height == 0 { return Err(PaintError::ZeroDimension); }
    let byte_count = u64::from(width) * u64::from(height) * 4;
    // wl_shm buffer offset, size and stride use signed 32-bit protocol fields.
    if byte_count > i32::MAX as u64 {
        return Err(PaintError::BufferTooLarge);
    }
    let mut bytes = vec![0u8; byte_count as usize];
    let outer_radius = style.border_radius;
    let border = style.border_size.min(width / 2).min(height / 2);
    for y in 0..height {
        for x in 0..width {
            if !inside_rounded(x, y, width, height, outer_radius) { continue; }
            let inside_inner = x >= border && x < width - border
                && y >= border && y < height - border
                && inside_rounded(
                    x.saturating_sub(border),
                    y.saturating_sub(border),
                    width - 2 * border,
                    height - 2 * border,
                    outer_radius.saturating_sub(border),
                );
            let color = if inside_inner {style.background} else {style.border};
            let offset = ((u64::from(y) * u64::from(width) + u64::from(x)) * 4) as usize;
            bytes[offset..offset + 4].copy_from_slice(&color.argb8888_le());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_existing_mako_palette_exactly() {
        assert_eq!(Color::parse_mako("#305747").unwrap(),
            Color {red:0x30,green:0x57,blue:0x47,alpha:255});
        assert_eq!(Color::parse_mako("#684148FF").unwrap().alpha,255);
        assert_eq!(Color::parse_mako("#715536").unwrap().blue,0x36);
        assert_eq!(Color::parse_mako("#gggggg"),Err("invalid hex digit"));
        assert!(Color::parse_mako("305747").is_err());
    }

    #[test]
    fn progress_color_matches_mako_strtok_and_ignores_extra_tokens(){
        let c=Color::parse_mako("#112233").unwrap();
        assert_eq!(parse_progress_color_mako(" \tOVER \t#112233 trailing"),
            Ok((ProgressBlend::Over,c)));
        assert_eq!(parse_progress_color_mako("SoUrCe #112233 trailing"),
            Ok((ProgressBlend::Source,c)));
        assert_eq!(parse_progress_color_mako("#112233 extra"),
            Ok((ProgressBlend::Over,c)));
        assert!(parse_progress_color_mako("source").is_err());
        assert!(parse_progress_color_mako("multiply #112233").is_err());
        assert!(parse_progress_color_mako("source\u{a0}#112233").is_err());
        assert!(parse_progress_color_mako("").is_err());
    }

    #[test]
    fn rounds_transparent_corners_and_preserves_white_border() {
        let style = RectangleStyle {
            background: Color::parse_mako("#3B4858").unwrap(),
            border: Color::parse_mako("#FFFFFF").unwrap(),
            border_radius: 3,
            border_size: 1,
        };
        let pixels = paint_rounded(12, 10, style).unwrap();
        assert_eq!(&pixels[0..4], &[0,0,0,0]);
        let edge = ((12 * 0 + 6) * 4) as usize;
        assert_eq!(&pixels[edge..edge+4], &[255,255,255,255]);
        let center = ((12 * 5 + 6) * 4) as usize;
        assert_eq!(&pixels[center..center+4], &[0x58,0x48,0x3B,0xFF]);
    }

    #[test]
    fn alpha_is_premultiplied_and_sizes_are_guarded() {
        assert_eq!(Color {red:200,green:100,blue:0,alpha:128}.argb8888_le(),
            [0,50,100,128]);
        let sample = RectangleStyle {
            background: Color {red:0,green:0,blue:0,alpha:255},
            border: Color {red:0,green:0,blue:0,alpha:255},
            border_radius:0,border_size:0
        };
        assert_eq!(paint_rounded(0,10,sample),Err(PaintError::ZeroDimension));
        assert_eq!(paint_rounded(100000,100000,sample),Err(PaintError::BufferTooLarge));
    }
}
