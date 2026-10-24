//! Headless-only compositor input-region acceptance. Two overlapping
//! independent layer-shell surfaces prove that a click outside the upper
//! surface's wl_region reaches the lower surface instead of being swallowed.
//! Only the private test compositor may invoke this fixture.
use posthorn::render::PixelFrame;
use posthorn::input::InputRect;
use posthorn::wayland::{LivePresenter,PopupLayer,PopupMargins,ScreenCorner};
use std::error::Error;
use std::io::Write;
use std::time::Duration;

fn opaque(width:u32,height:u32,color:[u8;3])->PixelFrame{
    let mut pixels=vec![0u8;width as usize*height as usize*4];
    for pixel in pixels.chunks_exact_mut(4){
        pixel.copy_from_slice(&[color[2],color[1],color[0],255]);
    }
    PixelFrame{width,height,stride:(width*4) as i32,pixels}
}

fn main()->Result<(),Box<dyn Error>>{
    if std::env::var("WLR_BACKENDS").ok().as_deref()!=Some("headless")
        || std::env::var("POSTHORN_HEADLESS_INPUT_TEST").ok().as_deref()!=Some("1"){
        return Err("input-region probe is restricted to isolated headless QA".into());
    }
    let mut args=std::env::args().skip(1);
    let ready=args.next().ok_or("missing readiness marker")?;
    let events=args.next().ok_or("missing input events marker")?;
    let flip_request=args.next().ok_or("missing region update request path")?;
    let flip_ack=args.next().ok_or("missing region update acknowledgment path")?;
    if args.next().is_some(){
        return Err("expected READY_PATH EVENTS_PATH FLIP_REQUEST FLIP_ACK".into());
    }
    let mut lower=LivePresenter::connect()?;
    let mut upper=LivePresenter::connect()?;
    let base=opaque(160,100,[0x2D,0xB3,0xC1]);
    let overlay=opaque(160,100,[0xE5,0x92,0x3C]);
    lower.update_frame_on_output(
        Some(&base),ScreenCorner::TopLeft,PopupMargins::uniform(0),
        PopupLayer::Top,"",
    )?;
    upper.update_native_frame_with_input_rectangles(
        Some(&overlay),ScreenCorner::TopLeft,PopupMargins::uniform(0),
        PopupLayer::Overlay,"",
        &[InputRect{x:80,y:0,width:80,height:100}],
    )?;
    let mut both_mapped=false;
    for _ in 0..80 {
        lower.poll_events()?;
        upper.poll_events()?;
        if lower.has_active_surface()&&upper.has_active_surface()
            &&lower.current_output_name().is_some()
            &&upper.current_output_name().is_some(){
            both_mapped=true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if !both_mapped{return Err("overlapping independent layer surfaces did not map".into());}
    let mut writer=std::fs::OpenOptions::new().create_new(true).append(true)
        .open(&events)?;
    std::fs::write(ready,"overlapping headless input regions mapped\n")?;
    let mut flipped=false;
    for _ in 0..600{
        lower.poll_events()?;
        upper.poll_events()?;
        if !flipped && std::path::Path::new(&flip_request).exists(){
            // Same logical size => LivePresenter.redraw_active, not layer
            // recreation. The compositor must apply the NEW input region
            // with the new frame in the same wl_surface.commit transaction.
            upper.update_native_frame_with_input_rectangles(
                Some(&overlay),ScreenCorner::TopLeft,PopupMargins::uniform(0),
                PopupLayer::Overlay,"",
                &[InputRect{x:0,y:0,width:80,height:100}],
            )?;
            flipped=true;
            std::fs::write(&flip_ack,"input region swapped on live surface\n")?;
        }
        for click in lower.take_clicks(){
            if click.button==273{
                writeln!(writer,"lower")?;
                writer.flush()?;
            }
        }
        for click in upper.take_clicks(){
            if click.button==273{
                writeln!(writer,"upper")?;
                writer.flush()?;
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn input_probe_uses_opaque_bgra_frames(){
        let frame=opaque(2,3,[0xE5,0x92,0x3C]);
        assert_eq!(frame.pixels.len(),24);
        assert!(frame.pixels.chunks_exact(4).all(|p|p==[0x3C,0x92,0xE5,255]));
    }
}
