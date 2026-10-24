//! Pixel-only acceptance probe for private Hyprland screenshot QA.
//! No screenshots are stored or uploaded by this binary.
use gdk_pixbuf::Pixbuf;
use std::error::Error;

/// Count approximate background RGB pixels, independent of alpha/row padding.
fn count_color(image:&Pixbuf,rgb:[u8;3])->Result<usize,Box<dyn Error>>{
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
    Ok(matched)
}

/// Inclusive bounding box of a screenshot color. Used to aim compositor
/// clicks at the *painted* lower edge, rather than a guessed font-dependent
/// notification height. The probe reads only private CI screenshot files.
fn color_bounds(
    image:&Pixbuf,rgb:[u8;3],
)->Result<Option<(usize,usize,usize,usize)>,Box<dyn Error>>{
    let channels=usize::try_from(image.n_channels())?;
    let stride=usize::try_from(image.rowstride())?;
    let width=usize::try_from(image.width())?;
    let height=usize::try_from(image.height())?;
    if channels<3||width==0||height==0{return Err("unsupported PNG pixel layout".into());}
    let data=image.read_pixel_bytes();
    let bytes=data.as_ref();
    let mut rect:Option<(usize,usize,usize,usize)>=None;
    for y in 0..height{
        for x in 0..width{
            let base=y.checked_mul(stride)
                .and_then(|r|x.checked_mul(channels).and_then(|c|r.checked_add(c)))
                .ok_or("pixel offset overflow")?;
            if base.checked_add(2).is_none_or(|end|end>=bytes.len()){
                return Err("truncated PNG pixel buffer".into());
            }
            if (0..3).all(|c|bytes[base+c].abs_diff(rgb[c])<=3){
                rect=Some(match rect{
                    Some((min_x,max_x,min_y,max_y))=>(
                        min_x.min(x),max_x.max(x),min_y.min(y),max_y.max(y)
                    ),
                    None=>(x,x,y,y),
                });
            }
        }
    }
    Ok(rect)
}

fn acceptance(mode:&str,current:usize,reference:Option<usize>,threshold:usize)->bool{
    match (mode,reference){
        ("present",None)=>current>=threshold,
        ("absent",None)=>current<threshold,
        // Compare against the actual user's desktop BEFORE the private
        // notification is drawn. This avoids a false pass/failure when a
        // wallpaper or existing application uses the QA background color.
        ("increase",Some(before))=>current.saturating_sub(before)>=threshold,
        ("restored",Some(before))=>current.abs_diff(before)<=threshold,
        _=>false,
    }
}

fn main()->Result<(),Box<dyn Error>>{
    let mut args=std::env::args().skip(1);
    let path=args.next().ok_or("missing screenshot path")?;
    let color=args.next().ok_or("missing #RRGGBB color")?;
    let minimum:usize=args.next().ok_or("missing pixel threshold")?.parse()?;
    let mode=args.next().ok_or("missing comparison mode")?;
    let reference_path=args.next();
    if args.next().is_some() || !matches!(
        (mode.as_str(),reference_path.is_some()),
        ("present"|"absent"|"bounds",false)|("increase"|"restored",true)
    ){
        return Err(
            "usage: qa_png_probe FILE #RRGGBB MIN_PIXELS present|absent|bounds|increase|restored [BASELINE]"
                .into()
        );
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
    let matched=count_color(&image,rgb)?;
    if mode=="bounds"{
        if matched<minimum {
            return Err("not enough matching pixels for a bounding box".into());
        }
        let (x0,x1,y0,y1)=color_bounds(&image,rgb)?
            .ok_or("no matching pixels for a bounding box")?;
        println!("{x0} {x1} {y0} {y1}");
        return Ok(());
    }
    let reference=match reference_path{
        Some(path)=>{
            let baseline=Pixbuf::from_file(path)?;
            if baseline.width()!=image.width() || baseline.height()!=image.height(){
                return Err("baseline and current screenshots have different geometry".into());
            }
            Some(count_color(&baseline,rgb)?)
        }
        None=>None,
    };
    println!(
        "matching pixels: {matched}; reference: {reference:?}; threshold: {minimum}; expectation: {mode}"
    );
    if acceptance(&mode,matched,reference,minimum){
        Ok(())
    }else{
        Err("screenshot color acceptance failed".into())
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn rgb_probe_counts_synthetic_argb_without_including_row_padding(){
        let image=Pixbuf::new(gdk_pixbuf::Colorspace::Rgb,true,8,8,6).unwrap();
        image.fill(0x193FA7FF);
        assert_eq!(count_color(&image,[0x19,0x3F,0xA7]).unwrap(),48);
        assert_eq!(count_color(&image,[0x1B,0x3F,0xA7]).unwrap(),48,
            "small screenshot color differences are tolerated");
        assert_eq!(count_color(&image,[0x55,0x55,0x55]).unwrap(),0);
    }

    #[test]
    fn screenshot_bounds_report_inclusive_min_max_coordinates(){
        let image=Pixbuf::new(gdk_pixbuf::Colorspace::Rgb,true,8,8,6).unwrap();
        image.fill(0x193FA7FF);
        assert_eq!(color_bounds(&image,[0x19,0x3F,0xA7]).unwrap(),Some((0,7,0,5)));
        assert_eq!(color_bounds(&image,[0x55,0x55,0x55]).unwrap(),None);
    }

    #[test]
    fn baseline_comparison_does_not_confuse_wallpaper_with_popup(){
        // The baseline may contain matching pixels; a raw present/absent
        // threshold cannot distinguish them from the popup.
        assert!(acceptance("increase",1800,Some(1200),500));
        assert!(!acceptance("increase",1250,Some(1200),500));
        assert!(acceptance("restored",1220,Some(1200),60));
        assert!(!acceptance("restored",1800,Some(1200),60));
        assert!(!acceptance("increase",1100,Some(1200),500));
        assert!(!acceptance("restored",1200,None,60));
        assert!(!acceptance("increase",1200,None,500));
    }
}
