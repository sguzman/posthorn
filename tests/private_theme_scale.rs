#![cfg(feature = "dbus-integration")]
//! Verify pinned Mako's theme scale scan on a real private Notify message.
//! No production notification name or workstation configuration is touched.

use std::collections::HashMap;

use posthorn::config::Config;
use posthorn::dbus::{BUS_NAME, OBJECT_PATH};
use posthorn::render::render_notification_group_with_icon_scale;
use posthorn::service::{open_private_service, PRIVATE_BUS_NAME};
use zbus::{zvariant::OwnedValue, Proxy};

#[tokio::test]
async fn theme_scale_from_later_category_survives_private_notify_and_raster()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let root = tempfile::tempdir()?;
    for (relative, rgba) in [
        ("16x16/apps", 0xFF0000FF),
        ("16x16/apps@2", 0x00FF00FF),
    ] {
        let path = root.path().join(relative).join("posthorn.png");
        std::fs::create_dir_all(path.parent().ok_or("missing image parent")?)?;
        let image = gdk_pixbuf::Pixbuf::new(
            gdk_pixbuf::Colorspace::Rgb, true, 8, 12, 12
        ).ok_or("unable to allocate icon test image")?;
        image.fill(rgba);
        image.savev(&path, "png", &[])?;
    }

    let config = Config::parse(&format!(
        "icon-path={}\nicons=1\nwidth=160\nheight=75\n\
         padding=4\nborder-size=0\nicon-location=left\nmax-icon-size=16\n",
        root.path().display()
    ))?;
    let (conn, runtime) = open_private_service(config.clone()).await?;
    let desktop = Proxy::new(
        &conn, PRIVATE_BUS_NAME, OBJECT_PATH, BUS_NAME
    ).await?;
    let id: u32 = desktop.call("Notify", &(
        "mako-theme-scale", 0u32, "posthorn", "Theme", "",
        Vec::<String>::new(), HashMap::<String, OwnedValue>::new(), -1i32
    )).await?;
    let note = runtime.store.lock().unwrap().active().iter()
        .find(|note| note.id == id).cloned()
        .ok_or("Notify did not create an active notification")?;
    assert_eq!(note.app_icon, "posthorn");

    // Keep popup output scale 1 in both cases; only the maximum attached
    // monitor's icon-search scale changes. The @2 suffix is on the later
    // category directory, which Mako's strchr(relative_path, '@') sees.
    let ordinary = render_notification_group_with_icon_scale(
        &note, &config.global, 0, 1, 1
    )?;
    let hidpi_icon = render_notification_group_with_icon_scale(
        &note, &config.global, 0, 1, 2
    )?;
    assert_eq!((ordinary.width, ordinary.height),
        (hidpi_icon.width, hidpi_icon.height),
        "theme selection must not resize the popup");
    let pixel = |frame: &posthorn::render::PixelFrame| {
        let at = (frame.height as usize / 2) * frame.stride as usize + 6 * 4;
        frame.pixels[at..at+4].to_vec()
    };
    assert_eq!(pixel(&ordinary), vec![0, 0, 255, 255],
        "1x selects the unscaled themed PNG through Notify");
    assert_eq!(pixel(&hidpi_icon), vec![0, 255, 0, 255],
        "2x must recognize @2 in the later category through Notify");
    Ok(())
}
