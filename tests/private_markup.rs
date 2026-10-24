#![cfg(feature = "dbus-integration")]
//! Mako v1.11.0 notification body markup acceptance through a private bus.
//! This test never requests the workstation's notification name.

use std::collections::HashMap;

use posthorn::config::Config;
use posthorn::dbus::{BUS_NAME, OBJECT_PATH};
use posthorn::render::render_notification;
use posthorn::service::{open_private_service, PRIVATE_BUS_NAME};
use zbus::{zvariant::OwnedValue, Proxy};

#[tokio::test]
async fn body_markup_enabled_and_disabled_are_distinct_on_the_dbus_render_path()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    // Mako validates sender markup separately from the final format template:
    // with markup=1, a valid span can supply a foreground color; with
    // markup=0 the same bytes must be painted as literal, escaped text.
    let config = Config::parse(
        "format=%b\nmarkup=1\nicons=0\nwidth=260\nheight=80\n\
         padding=6\nborder-size=0\nbackground-color=#000000\n\
         text-color=#ffffff\nfont=Sans 18\n",
    )?;
    let (conn, runtime) = open_private_service(config.clone()).await?;
    let desktop = Proxy::new(
        &conn, PRIVATE_BUS_NAME, OBJECT_PATH, BUS_NAME
    ).await?;
    let body = "<span foreground='#ff0000'>VISIBLE</span>";
    let id: u32 = desktop.call("Notify", &(
        "markup-wire", 0u32, "", "Sender title", body,
        Vec::<String>::new(), HashMap::<String, OwnedValue>::new(), -1i32
    )).await?;
    let note = runtime.store.lock().unwrap().active().iter()
        .find(|item| item.id == id).cloned()
        .ok_or("private Notify did not retain notification")?;
    assert_eq!(note.body, body, "the D-Bus input must not pre-strip markup");

    let markup_frame = render_notification(&note, &config.global)?;
    let mut plain = config.global.clone();
    plain.insert("markup".into(), "0".into());
    let plain_frame = render_notification(&note, &plain)?;

    // Cairo ARGB32 uses little-endian BGRA bytes on Linux. Look for a
    // strongly red glyph pixel, not a brittle exact antialiased screenshot.
    // Do not mistake subpixel antialiasing fringes in white text for a
    // red glyph: only count saturated red pixels with little green/blue.
    let saturated_red_pixels = |pixels: &[u8]| pixels.chunks_exact(4).filter(|px| {
        let (blue, green, red) = (px[0], px[1], px[2]);
        red > 220 && green < 60 && blue < 60 && px[3] > 220
    }).count();
    let marked_red = saturated_red_pixels(&markup_frame.pixels);
    let plain_red = saturated_red_pixels(&plain_frame.pixels);
    assert!(marked_red > 10,
        "valid sender Pango markup must fill multiple glyph pixels red; got {marked_red}");
    assert!(marked_red > plain_red.saturating_mul(4),
        "markup=0 must not color text red: marked={marked_red}, plain={plain_red}");

    // Malformed markup is a valid notification body. Mako escapes it
    // rather than discarding the message or failing the render.
    let malformed: u32 = desktop.call("Notify", &(
        "markup-wire", 0u32, "", "Second title", "<span>unfinished",
        Vec::<String>::new(), HashMap::<String, OwnedValue>::new(), -1i32
    )).await?;
    let malformed_note = runtime.store.lock().unwrap().active().iter()
        .find(|item| item.id == malformed).cloned()
        .ok_or("malformed sender markup unexpectedly lost the notification")?;
    assert!(render_notification(&malformed_note, &config.global).is_ok());
    Ok(())
}
