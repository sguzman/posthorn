//! Purely synthetic theme fixture; no personal user data or config paths.
use posthorn::config::{Config, Traits};
use posthorn::notification::{NotificationStore, Request};
use posthorn::render::render_notification;

#[test]
fn four_notification_categories_produce_four_different_pixel_backgrounds() {
    let config = Config::parse(
        "width=160\nheight=80\nborder-size=0\npadding=8\nfont=Sans 10\n         background-color=#3B4858\n         [category=success]\nbackground-color=#305747\n         [category=failure]\nbackground-color=#684148\n         [category=warning]\nbackground-color=#715536\n"
    ).unwrap();
    for (category, rgb) in [
        ("normal", [0x58u8,0x48,0x3B,0xFF]),
        ("success", [0x47,0x57,0x30,0xFF]),
        ("failure", [0x48,0x41,0x68,0xFF]),
        ("warning", [0x36,0x55,0x71,0xFF]),
    ] {
        let mut store = NotificationStore::new(1);
        store.push(Request {category: category.into(), summary: "A".into(), ..Request::default()});
        let note = store.active().front().unwrap();
        let modes = ["default"];
        let traits = Traits {
            app_name: &note.app_name, app_icon: &note.app_icon,
            category: &note.category, desktop_entry: &note.desktop_entry,
            summary: &note.summary, body: &note.body, urgency: "normal",
            anchor: "top-right", output: "", modes: &modes,
            requested_timeout_ms: note.requested_timeout_ms,
            actionable: false, group_index: -1, hidden: false,
        };
        let frame = render_notification(note, &config.style_for(&traits)).unwrap();
        // Mako's height setting is a maximum; select a pixel inside
        // the measured shrink-wrapped frame, not at a fixed y=65.
        let idx = ((frame.height as usize-4) * frame.stride as usize) + 130 * 4;
        assert_eq!(&frame.pixels[idx..idx+4], &rgb[..], "wrong pixels for {category}");
    }
}
