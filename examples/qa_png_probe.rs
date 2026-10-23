//! Pixel-only acceptance probe for private Hyprland screenshot QA.
//! No screenshots are stored or uploaded by this binary.
use gdk_pixbuf::Pixbuf;
use std::error::Error;

fn main()->Result<(),Box<dyn Error>>{
    let mut args=std::env::args().skip(1);
    let path=args.next().ok_or("missing screenshot path")?;
    let color=args.next().ok_or("missing #RRGGBB color")?;
    let minimum:usize=args.next().ok_or("missing pixel threshold")?.parse()?;
    let mode=args.next().ok_or("missing present|absent mode")?;
    if args.next().is_some() || !["present","absent"].contains(&mode.as_str()) {
        return Err("usage: qa_png_probe FILE #RRGGBB MIN_PIXELS present|absent".into());
    }
    let hex=color.strip_prefix('#').ok_or("color must start with #")?;
    if hex.len()!=6 || !hex.bytes().all(|b|b.is_ascii_hexdigit()){
        return Err("color must be six hexadecimal digits".into());
    }
    let rgb=[
        u8::from_str_radix(&hex[0..2],16)?,
        u8::from_str_radix(&hex[2..4],16)?,
        u8::from_str_radix(&hex[4..6],16)?,
    ];
    let image=Pixbuf::from_file(&path)?;
    let channels=usize::try_from(image.n_channels())?;
    let stride=usize::try_from(image.rowstride())?;
    let width=usize::try_from(image.width())?;
    let height=usize::try_from(image.height())?;
    if channels<3 || width==0 || height==0 {
        return Err("unsupported PNG pixel layout".into());
    }
    let bytes=image.read_pixel_bytes();
    let raw=bytes.as_ref();
    let mut matched=0usize;
    for y in 0..height {
        for x in 0..width {
            let offset=y.checked_mul(stride)
                .and_then(|base|x.checked_mul(channels).and_then(|shift|base.checked_add(shift)))
                .ok_or("pixel offset overflow")?;
            if offset.checked_add(2).is_none_or(|last|last>=raw.len()){
                return Err("truncated PNG pixel buffer".into());
            }
            if (0..3).all(|channel|raw[offset+channel].abs_diff(rgb[channel])<=3){
                matched+=1;
            }
        }
    }
    println!("matching pixels: {matched}; threshold: {minimum}; expectation: {mode}");
    match mode.as_str() {
        "present" if matched>=minimum=>Ok(()),
        "absent" if matched<minimum=>Ok(()),
        _=>Err("screenshot color acceptance failed".into()),
    }
}
