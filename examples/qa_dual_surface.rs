//! Headless-only dual Wayland layer-shell acceptance probe. No D-Bus ownership,
//! user desktop configuration, or installed notification daemon is touched.
//! Each independent presenter has its own Wayland connection and output events.
use posthorn::render::PixelFrame;
use posthorn::wayland::{LivePresenter,PopupLayer,PopupMargins,ScreenCorner};
use std::error::Error;
use std::time::Duration;

fn solid_frame(width:u32,height:u32,rgb:[u8;3])->PixelFrame{
    let mut pixels=Vec::with_capacity(width as usize*height as usize*4);
    for _ in 0..width*height{
        pixels.extend_from_slice(&[rgb[2],rgb[1],rgb[0],0xff]);
    }
    PixelFrame{width,height,stride:(width*4) as i32,pixels}
}

fn main()->Result<(),Box<dyn Error>>{
    let mut args=std::env::args().skip(1);
    let marker=args.next().ok_or("missing readiness marker path")?;
    if args.next().is_some(){return Err("expected one marker path".into());}
    let mut upper=LivePresenter::connect()?;
    let mut lower=LivePresenter::connect()?;
    let orange=solid_frame(126,72,[0xE5,0x92,0x3C]);
    let blue=solid_frame(112,68,[0x2D,0xB3,0xC1]);
    upper.update_frame_on_output(
        Some(&orange),ScreenCorner::TopLeft,PopupMargins::uniform(15),
        PopupLayer::Top,"",
    )?;
    lower.update_frame_on_output(
        Some(&blue),ScreenCorner::BottomRight,PopupMargins::uniform(15),
        PopupLayer::Top,"",
    )?;
    let mut mapped=false;
    for _ in 0..80{
        upper.poll_events()?;
        lower.poll_events()?;
        if upper.has_active_surface()&&lower.has_active_surface()
            &&upper.current_output_name().is_some()
            &&lower.current_output_name().is_some(){
            mapped=true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if !mapped{return Err("both independent surfaces must receive compositor enters".into());}
    std::fs::write(marker,"two mapped independent compositor surfaces\n")?;
    // Keep both surfaces visible while the parent headless Sway test captures
    // a screenshot. The parent checks BOTH solid colors in that same image.
    for _ in 0..80{
        upper.poll_events()?;
        lower.poll_events()?;
        std::thread::sleep(Duration::from_millis(25));
    }
    println!("Two independent Wayland layer surfaces mapped and held");
    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn dual_probe_uses_valid_premultiplied_opaque_bgra_pixels(){
        let frame=solid_frame(2,3,[0xE5,0x92,0x3C]);
        assert_eq!((frame.width,frame.height,frame.stride),(2,3,8));
        assert_eq!(frame.pixels.len(),24);
        assert!(frame.pixels.chunks_exact(4)
            .all(|pixel|pixel==[0x3C,0x92,0xE5,0xff]));
    }
}
