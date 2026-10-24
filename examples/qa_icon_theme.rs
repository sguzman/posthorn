//! Write deterministic differently-shaped nominal-16 icon theme assets
//! for private headless mixed-DPI compositor acceptance. No system icon
//! locations are touched, and the tool refuses non-headless invocation.
use gdk_pixbuf::{Colorspace,Pixbuf};
use std::error::Error;
use std::path::Path;

fn write_theme(root:&Path)->Result<(),Box<dyn Error>>{
    for (subdir,width,height,rgba) in [
        ("16x16/apps",3,3,0xFF0000FF),
        ("16x16@2/apps",12,57,0x00FF00FF),
    ]{
        let path=root.join(subdir).join("posthorn-qa.png");
        std::fs::create_dir_all(path.parent().ok_or("invalid fixture path")?)?;
        let pixels=Pixbuf::new(Colorspace::Rgb,true,8,width,height)
            .ok_or("cannot allocate icon fixture")?;
        pixels.fill(rgba);
        pixels.savev(path,"png",&[])?;
    }
    Ok(())
}

fn main()->Result<(),Box<dyn Error>>{
    if std::env::var("WLR_BACKENDS").ok().as_deref()!=Some("headless")
       || std::env::var("POSTHORN_HEADLESS_ICON_TEST").ok().as_deref()!=Some("1"){
        return Err("icon fixture is restricted to private headless QA".into());
    }
    let mut args=std::env::args().skip(1);
    let path=args.next().ok_or("missing private output directory")?;
    if args.next().is_some(){return Err("expected one output directory".into());}
    write_theme(Path::new(&path))
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn fixture_icons_have_distinct_exact_match_scales_and_aspects(){
        let directory=tempfile::tempdir().unwrap();
        write_theme(directory.path()).unwrap();
        let one=Pixbuf::from_file(directory.path()
            .join("16x16/apps/posthorn-qa.png")).unwrap();
        let two=Pixbuf::from_file(directory.path()
            .join("16x16@2/apps/posthorn-qa.png")).unwrap();
        assert_eq!((one.width(),one.height()),(3,3));
        assert_eq!((two.width(),two.height()),(12,57));
    }
}
