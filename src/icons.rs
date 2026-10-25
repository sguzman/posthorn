//! Mako-compatible app-icon lookup and image decoding.
//!
//! Accepts absolute paths, file:// URLs, themed names, freedesktop pixmaps,
//! and decoded image-data hints. Exact SVG theme fidelity remains MVP work;
//! sender-provided themed names cannot escape search roots.

use std::fs;
use std::path::{Path,PathBuf};
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use gdk_pixbuf::Pixbuf;
use crate::config::Options;
use crate::notification::Notification;

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct IconPixels {
    pub width:u32,
    pub height:u32,
    /// Premultiplied little-endian ARGB8888 as Cairo and wl_shm expect.
    pub bgra:Vec<u8>,
}

/// Freedesktop image-data hint: (width, height, rowstride, alpha,
/// bits-per-sample, channels, raw bytes). Reject malformed sender data before
/// allocating a Cairo image or performing any indexing.
pub fn decode_raw_image(
    width:i32,height:i32,rowstride:i32,has_alpha:bool,
    bits_per_sample:i32,channels:i32,bytes:&[u8],
) -> Option<IconPixels> {
    let (w,h,stride,channels)=(
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        usize::try_from(rowstride).ok()?,
        usize::try_from(channels).ok()?,
    );
    if w==0 || h==0 || w>4096 || h>4096
        || u64::from(w)*u64::from(h)>4_194_304
        || bits_per_sample!=8
        || channels != if has_alpha {4}else{3}
    {return None;}
    let row=(w as usize).checked_mul(channels)?;
    let last=(h as usize-1).checked_mul(stride)?.checked_add(row)?;
    if stride<row||last>bytes.len()||bytes.len()>16*1024*1024{
        return None;
    }
    let mut bgra=vec![0u8;(w as usize).checked_mul(h as usize)?.checked_mul(4)?];
    for y in 0..h as usize {
        let row_base=y.checked_mul(stride)?;
        for x in 0..w as usize {
            let pos=row_base.checked_add(x.checked_mul(channels)?)?;
            let rgba=bytes.get(pos..pos+channels)?;
            let a=if has_alpha{rgba[3]}else{255};
            let premultiply=|color:u8|((u16::from(color)*u16::from(a)+127)/255) as u8;
            let dest=(y*w as usize+x)*4;
            bgra[dest..dest+4].copy_from_slice(&[
                premultiply(rgba[2]),premultiply(rgba[1]),premultiply(rgba[0]),a,
            ]);
        }
    }
    Some(IconPixels{width:w,height:h,bgra})
}

fn hex(b:u8)->Option<u8>{
    match b {b'0'..=b'9'=>Some(b-b'0'),b'a'..=b'f'=>Some(b-b'a'+10),
        b'A'..=b'F'=>Some(b-b'A'+10),_=>None}
}

fn url_decode(input:&str)->Option<Vec<u8>> {
    let bytes=input.as_bytes();
    let mut decoded=Vec::with_capacity(bytes.len());
    let mut i=0;
    while i<bytes.len(){
        if bytes[i]==b'%' && i+2<bytes.len(){
            if let (Some(hi),Some(lo))=(hex(bytes[i+1]),hex(bytes[i+2])){
                let value=(hi<<4)|lo;
                // Mako's icon.c:url_decode writes into a NUL-terminated
                // C filename. A decoded %00 ends that filename; the
                // trailing URI bytes never reach gdk_pixbuf_new_from_file.
                // Rust paths cannot contain NUL, so retain the exact
                // prefix instead of rejecting the entire source URI.
                if value==0 { break; }
                decoded.push(value);
                i+=3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i+=1;
    }
    Some(decoded)
}

fn valid_icon_name(name:&str)->bool {
    !name.is_empty()&&name.len()<=1024&&name.bytes().all(|b|
        b.is_ascii_alphanumeric()||[b'-',b'_',b'.'].contains(&b))
}

fn icon_files(dir:&Path,name:&str)->Vec<PathBuf>{
    let Ok(entries)=fs::read_dir(dir) else {return Vec::new()};
    let prefix=format!("{name}.");
    entries.flatten().filter_map(|e|{
        let path=e.path();
        let good=path.is_file()&&path.file_name()
            .is_some_and(|n|n.to_string_lossy().starts_with(&prefix));
        good.then_some(path)
    }).collect()
}

/// Mako's glob is root/*/*/name.*. It reads strtol() from the first
/// directory and falls back to the second only when the first is unsized.
/// After choosing where to read the size, strchr() searches the ENTIRE
/// remaining relative path for the first '@': later category directories
/// and even a filename extension can supply the scale.
fn signed_size_prefix(component:&str)->Option<i32>{
    // Upstream uses (int32_t)strtol(component, NULL, 10). A completely
    // nonnumeric component is zero, but a NEGATIVE nonzero first component
    // still prevents fallback to the second directory. strtol itself
    // parses a signed 64-bit prefix and then narrows to int32 on Linux.
    let text=component.trim_start_matches([' ','\t','\n','\r','\x0b','\x0c']);
    let (negative,rest)=if let Some(x)=text.strip_prefix('-'){(true,x)}
        else{(false,text.strip_prefix('+').unwrap_or(text))};
    let digits:String=rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty(){return Some(0);}
    let unsigned=digits.parse::<u64>().ok()?;
    let wide=if negative{
        if unsigned==(1u64<<63){i64::MIN}
        else if unsigned<=i64::MAX as u64{-(unsigned as i64)}
        else{return None;}
    }else{
        i64::try_from(unsigned).ok()?
    };
    Some(wide as i32)
}
fn theme_icon_size(first:&str,second:&str,filename:&str)->Option<(u32,u32)>{
    let from_first=signed_size_prefix(first);
    let (signed,starts_at_first)=match from_first{
        Some(0)|None=>(signed_size_prefix(second)?,false),
        Some(size)=>(size,true),
    };
    let nominal=u32::try_from(signed).ok().filter(|size|*size>0)?;
    // icon.c's strchr(relative_path, '@') begins at whichever directory
    // strtol selected, then searches through the REST of the path.
    // In particular, root/64x64/apps@2/name.png is a 2x asset, whereas
    // root/apps@2/64x64/name.png is 1x: the first component gets skipped.
    let suffix=if starts_at_first {
        [first,second,filename].iter()
            .find_map(|part|part.split_once('@').map(|(_,tail)|tail))
    }else{
        [second,filename].iter()
            .find_map(|part|part.split_once('@').map(|(_,tail)|tail))
    };
    let scale=match suffix {
        Some(tail)=>signed_size_prefix(tail)
            .and_then(|size|u32::try_from(size).ok()).unwrap_or(0),
        None=>1,
    };
    Some((nominal,scale))
}

/// Resolve the icon using the same two-tier theme lookup as Mako. The output
/// scale selects sharper assets without changing the logical icon dimensions.
pub fn resolve_icon_scaled(
    name:&str,path_spec:&str,max_size:u32,output_scale:u32
)->Option<PathBuf>{
    if name.starts_with('/') {return Some(PathBuf::from(name));}
    if let Some(uri)=name.strip_prefix("file://"){
        // Mako strips the scheme and URL-decodes the remainder verbatim.
        // It does not require an absolute path: file://relative.png is
        // resolved against the daemon's working directory by GdkPixbuf.
        // Mako's URL decoder writes raw Linux filename bytes. Requiring
        // valid UTF-8 would discard otherwise valid on-disk filenames.
        return Some(PathBuf::from(OsString::from_vec(url_decode(uri)?)));
    }
    if !valid_icon_name(name){return None;}
    let max_scale=output_scale.max(1);
    let physical_cap=max_size.saturating_mul(max_scale);
    let themes=path_spec.split(':')
        .filter(|x|!x.is_empty())
        .map(PathBuf::from)
        .chain(std::iter::once(PathBuf::from("/usr/share/icons/hicolor")));
    for root in themes {
        let Ok(first_levels)=fs::read_dir(&root) else{continue};
        let mut exact=Vec::new();
        let mut smaller=Vec::new();
        for first in first_levels.flatten(){
            if !first.path().is_dir(){continue;}
            let first_name=first.file_name().to_string_lossy().into_owned();
            let Ok(second_levels)=fs::read_dir(first.path()) else{continue};
            for second in second_levels.flatten(){
                let path=second.path();
                if !path.is_dir(){continue;}
                let second_name=second.file_name().to_string_lossy().into_owned();
                for file in icon_files(&path,name){
                    let Some(file_name)=file.file_name() else{continue};
                    let file_name=file_name.to_string_lossy();
                    let Some((size,scale))=
                        theme_icon_size(&first_name,&second_name,&file_name)
                        else{continue};
                    if size==max_size && scale==max_scale {
                        exact.push(file);
                    }else if size<physical_cap {
                        smaller.push((size,file));
                    }
                }
            }
        }
        exact.sort();
        if let Some(path)=exact.into_iter().next(){return Some(path);}
        // Mako accepts an undersized candidate, not an arbitrary oversized
        // nearest match. Preserve deterministic ordering for equal sizes.
        smaller.sort_by(|(a,pa),(b,pb)|b.cmp(a).then_with(||pa.cmp(pb)));
        if let Some((_,path))=smaller.into_iter().next(){return Some(path);}
    }
    icon_files(Path::new("/usr/share/pixmaps"),name).into_iter().next()
}

pub fn resolve_icon(name:&str,path_spec:&str,max_size:u32)->Option<PathBuf>{
    resolve_icon_scaled(name,path_spec,max_size,1)
}

/// Read a GdkPixbuf into a bounded, Cairo-compatible ARGB32 buffer.
pub fn decode_icon(path:&Path)->Option<IconPixels>{
    let image=Pixbuf::from_file(path).ok()?;
    let (w,h)=(u32::try_from(image.width()).ok()?,u32::try_from(image.height()).ok()?);
    if w==0||h==0||w>4096||h>4096||u64::from(w)*u64::from(h)>4_194_304{
        return None;
    }
    let channels=usize::try_from(image.n_channels()).ok()?;
    let stride=usize::try_from(image.rowstride()).ok()?;
    if ![3,4].contains(&channels)||image.bits_per_sample()!=8{return None;}
    let source=image.read_pixel_bytes();
    let source: &[u8]=source.as_ref();
    let mut out=vec![0;usize::try_from(u64::from(w)*u64::from(h)*4).ok()?];
    let row_bytes=(w as usize).checked_mul(channels)?;
    for y in 0..h as usize {
        let start=y.checked_mul(stride)?;
        let src=source.get(start..start.checked_add(row_bytes)?)?;
        for x in 0..w as usize{
            let p=&src[x*channels..(x+1)*channels];
            let a=if channels==4{p[3]}else{255};
            let to_bg=|v:u8| ->u8{
                ((u16::from(v)*u16::from(a)+127)/255) as u8
            };
            let at=(y*w as usize+x)*4;
            out[at..at+4].copy_from_slice(&[to_bg(p[2]),to_bg(p[1]),to_bg(p[0]),a]);
        }
    }
    Some(IconPixels{width:w,height:h,bgra:out})
}

fn max_icon_size(style:&Options)->u32{
    style.get("max-icon-size")
        .and_then(|text|crate::config::parse_nonnegative_mako(text))
        .unwrap_or(64)
}

pub fn load_note_icon(note:&Notification,style:&Options)->Option<IconPixels>{
    load_note_icon_scaled(note,style,1)
}

pub fn load_note_icon_scaled(
    note:&Notification,style:&Options,output_scale:u32
)->Option<IconPixels>{
    if style.get("icons").is_some_and(|v|v=="0"||v.eq_ignore_ascii_case("false")){
        return None;
    }
    // image-data has higher precedence than image-path and app_icon.
    if let Some(image)=&note.image_data {return Some(image.clone());}
    if note.app_icon.is_empty(){return None;}
    let max_size=max_icon_size(style);
    let path=resolve_icon_scaled(&note.app_icon,
        style.get("icon-path").map(String::as_str).unwrap_or(""),
        max_size,output_scale)?;
    decode_icon(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freedesktop_raw_images_premultiply_alpha_and_honor_row_padding() {
        let image=decode_raw_image(1,2,8,true,8,4,&[
            255,128,0,128,0,0,0,0, 0,64,255,255,
        ]).unwrap();
        assert_eq!((image.width,image.height),(1,2));
        assert_eq!(&image.bgra[0..4],&[0,64,128,128]);
        assert_eq!(&image.bgra[4..8],&[255,64,0,255]);
    }

    #[test]
    fn malformed_sender_image_metadata_is_bounded_and_rejected() {
        assert!(decode_raw_image(1,1,-1,true,8,4,&[0;4]).is_none());
        assert!(decode_raw_image(50000,1,200000,true,8,4,&[0;4]).is_none());
        assert!(decode_raw_image(2,2,8,true,8,4,&[0;9]).is_none());
        assert!(decode_raw_image(1,1,3,true,8,4,&[0;4]).is_none());
        assert!(decode_raw_image(1,1,4,true,16,4,&[0;4]).is_none());
        assert!(decode_raw_image(1,1,4,true,8,3,&[0;4]).is_none());
    }

    #[test]
    fn uppercase_false_disables_inline_and_file_icons() {
        let mut store=crate::notification::NotificationStore::new(0);
        store.push(crate::notification::Request{
            app_icon:"/nonexistent/picture.png".into(),
            image_data:Some(IconPixels{
                width:1,height:1,bgra:vec![10,20,30,255],
            }),
            ..crate::notification::Request::default()
        });
        let note=store.active().front().unwrap();
        let style=crate::config::Config::parse("icons=FALSE\n").unwrap().global;
        assert!(load_note_icon(note,&style).is_none());
        let enabled=crate::config::Config::parse("icons=TRUE\n").unwrap().global;
        assert_eq!(load_note_icon(note,&enabled).unwrap().width,1);
    }

    #[test]
    fn rejects_traversal_and_decodes_file_uri() {
        assert!(!valid_icon_name("../payload"));
        assert!(resolve_icon("../payload","/usr/share/icons",64).is_none());
        assert!(resolve_icon("bad/file","/usr/share/icons",64).is_none());
        assert_eq!(resolve_icon("file:///tmp/hello%20world.png","",64),
            Some(PathBuf::from("/tmp/hello world.png")));
        assert_eq!(resolve_icon("file:///tmp/prefix.png%00ignored","",64),
            Some(PathBuf::from("/tmp/prefix.png")));
        assert!(resolve_icon("file://%00hidden","",64).is_none(),
            "empty C filename must not become a directory");
        assert!(resolve_icon("file:///%00hidden","",64).is_some(),
            "root slash is a nonempty truncated filename in Mako");
    }

    #[test]
    fn icon_theme_lookup_uses_mako_signed_numeric_parser(){
        let mut style=Options::new();
        assert_eq!(max_icon_size(&style),64);
        style.insert("max-icon-size".into()," \t+16".into());
        assert_eq!(max_icon_size(&style),16);
        style.insert("max-icon-size".into(),"48".into());
        assert_eq!(max_icon_size(&style),48);
        style.insert("max-icon-size".into(),"2147483648".into());
        assert_eq!(max_icon_size(&style),64);
    }

    #[test]
    fn mako_file_url_accepts_relative_paths_after_scheme() {
        assert_eq!(resolve_icon("file://relative.png","",64),
            Some(PathBuf::from("relative.png")));
        assert_eq!(resolve_icon("file://assets/my%20icon.png","",64),
            Some(PathBuf::from("assets/my icon.png")));
        assert_eq!(resolve_icon("file:///tmp/my%20icon.png","",64),
            Some(PathBuf::from("/tmp/my icon.png")));
        assert_eq!(resolve_icon("file://../one%2Ftwo.png","",64),
            Some(PathBuf::from("../one/two.png")));
        // Relative unprefixed names still go through the confined theme
        // resolver; no general directory-traversal exemption is introduced.
        assert!(resolve_icon("../one/two.png","",64).is_none());
    }

    #[test]
    fn file_urls_preserve_non_utf8_linux_filename_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let path=resolve_icon("file://images/name%FF.png","",64).unwrap();
        assert_eq!(path.as_os_str().as_bytes(),b"images/name\xff.png");
        assert!(path.to_str().is_none());
        // The upstream decoder also passes through malformed percent
        // escapes as literal '%' rather than treating them as fatal.
        let literal=resolve_icon("file://images/100%broken.png","",64).unwrap();
        assert_eq!(literal.as_os_str().as_bytes(),b"images/100%broken.png");
        // Mako's decoded filename is a C string; a percent-encoded NUL
        // ends it, even when more apparent filename characters follow.
        assert_eq!(resolve_icon("file://images/name.png%00hidden.png","",64),
            Some(PathBuf::from("images/name.png")));
        assert_eq!(resolve_icon("file://images/%00hidden.png","",64),
            Some(PathBuf::from("images/")));
        assert_eq!(resolve_icon("file://assets/bad%00%FFsuffix","",64),
            Some(PathBuf::from("assets/bad")));
    }

    #[test]
    fn themed_lookup_respects_size_and_safe_paths() {
        let root=tempfile::tempdir().unwrap();
        for size in [16,48,128] {
            let folder=root.path().join(format!("{size}x{size}/apps"));
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("posthorn.png"),b"fixture").unwrap();
        }
        assert_eq!(resolve_icon("posthorn",root.path().to_str().unwrap(),64),
            Some(root.path().join("48x48/apps/posthorn.png")));
        assert_eq!(resolve_icon("posthorn",root.path().to_str().unwrap(),16),
            Some(root.path().join("16x16/apps/posthorn.png")));
    }

    #[test]
    fn scaled_icon_lookup_prefers_exact_hidpi_and_supports_reverse_layout() {
        let root=tempfile::tempdir().unwrap();
        for rel in ["48x48/apps","64x64/apps","64x64@2/apps",
                    "apps/96x96","apps/128x128"] {
            let dir=root.path().join(rel);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("posthorn.png"),b"fixture").unwrap();
        }
        let root=root.path().to_str().unwrap();
        assert_eq!(resolve_icon_scaled("posthorn",root,64,2),
            Some(Path::new(root).join("64x64@2/apps/posthorn.png")));
        assert_eq!(resolve_icon_scaled("posthorn",root,64,1),
            Some(Path::new(root).join("64x64/apps/posthorn.png")));
        let reversed=tempfile::tempdir().unwrap();
        let folder=reversed.path().join("apps/96x96");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("posthorn.png"),b"fixture").unwrap();
        assert_eq!(resolve_icon_scaled("posthorn",reversed.path().to_str().unwrap(),64,2),
            Some(folder.join("posthorn.png")));
    }

    #[test]
    fn mako_icon_scale_scans_the_remaining_relative_path(){
        // icon.c uses strchr(relative_path, '@'), not strchr(size_part).
        assert_eq!(theme_icon_size("64x64","apps@2","posthorn.png"),Some((64,2)));
        assert_eq!(theme_icon_size("apps@2","64x64","posthorn.png"),Some((64,1)));
        assert_eq!(theme_icon_size("64x64@2","apps","posthorn.png"),Some((64,2)));
        assert_eq!(theme_icon_size("apps","64x64@2","posthorn.png"),Some((64,2)));
        assert_eq!(theme_icon_size("64x64","apps","posthorn.png@2"),Some((64,2)));
        assert_eq!(theme_icon_size("64x64@3","apps@2","posthorn.png"),Some((64,3)));
        assert_eq!(theme_icon_size("64x64@garbage","apps","posthorn.png"),Some((64,0)));
        assert_eq!(theme_icon_size("64x64@+2stuff","apps","posthorn.png"),Some((64,2)));
        assert_eq!(theme_icon_size("apps","+64x64@2","posthorn.png"),Some((64,2)));
        // Once the first parse fails, scanning starts from the second dir.
        assert_eq!(theme_icon_size("apps@3","64x64@2","posthorn.png"),Some((64,2)));
        assert_eq!(theme_icon_size("64x64","apps@wrong","posthorn.png@2"),
            Some((64,0)), "strchr stops at the first @, even if its suffix is invalid");
    }

    #[test]
    fn signed_mako_icon_directory_sizes_do_not_fall_back_from_negatives(){
        assert_eq!(signed_size_prefix("64x64"),Some(64));
        assert_eq!(signed_size_prefix(" -64x64"),Some(-64));
        assert_eq!(signed_size_prefix("+64x64"),Some(64));
        assert_eq!(signed_size_prefix("apps"),Some(0));
        assert_eq!(signed_size_prefix("4294967360x64"),Some(64),
            "Mako narrows its 64-bit strtol result to a 32-bit icon size");
        assert_eq!(signed_size_prefix("9223372036854775808x64"),None,
            "strtol reports overflow rather than narrowing");
        assert_eq!(theme_icon_size("-1","64x64","posthorn.png"),None,
            "a nonzero signed first-directory size prevents fallback");
        assert_eq!(theme_icon_size("apps","64x64","posthorn.png"),Some((64,1)));
        assert_eq!(theme_icon_size("4294967360","apps","posthorn.png"),Some((64,1)));
    }

    #[test]
    fn negative_first_directory_does_not_claim_a_valid_second_size(){
        let root=tempfile::tempdir().unwrap();
        let bad=root.path().join("-1/64x64/posthorn.png");
        std::fs::create_dir_all(bad.parent().unwrap()).unwrap();
        std::fs::write(&bad,b"fixture").unwrap();
        // The first component is negative, not zero; Mako skips this
        // candidate rather than examining the second 64x64 component.
        assert_eq!(resolve_icon("posthorn",root.path().to_str().unwrap(),64),None);
    }

    #[test]
    fn icon_lookup_accepts_later_scale_suffix_when_size_is_first(){
        let root=tempfile::tempdir().unwrap();
        // The second component is still searched by strchr() after the
        // nominal size was read from the first component.
        for rel in ["64x64/apps","64x64/apps@2"] {
            let dir=root.path().join(rel);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("posthorn.png"),b"fixture").unwrap();
        }
        let root=root.path();
        assert_eq!(resolve_icon_scaled("posthorn",root.to_str().unwrap(),64,1),
            Some(root.join("64x64/apps/posthorn.png")));
        assert_eq!(resolve_icon_scaled("posthorn",root.to_str().unwrap(),64,2),
            Some(root.join("64x64/apps@2/posthorn.png")));
        // The glob also permits filename suffixes that contain an @.
        let second=tempfile::tempdir().unwrap();
        let dir=second.path().join("64x64/apps");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("posthorn.png@2"),b"fixture").unwrap();
        assert_eq!(resolve_icon_scaled(
            "posthorn",second.path().to_str().unwrap(),64,2
        ),Some(dir.join("posthorn.png@2")));
    }

    #[test]
    fn malformed_pixel_data_is_not_accepted_as_an_icon() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("broken.png");
        fs::write(&path,b"not an image").unwrap();
        assert!(decode_icon(&path).is_none());
    }
}
