//! Public synthetic fixture: deliberately no private workstation paths/data.
use posthorn::config::{Config, Traits};
use posthorn::notification::{CloseReason, NotificationStore, Request};

#[test]
fn config_accepts_a_mako_category_theme() {
    let fixture = "default-timeout=7000\nignore-timeout=1\nwidth=440\nheight=180\npadding=14\nbackground-color=#3B4858\ntext-color=#E0E5EC\nborder-color=#FFFFFF\nborder-radius=16\nfont=Atkinson Hyperlegible Next 13\nmarkup=1\nformat=<span font_desc=\"Luciole Bold 15\">%s</span>\\n%b\n[category=success]\non-notify=exec /usr/bin/pw-play /tmp/success.wav\nbackground-color=#305747\n[category=failure]\non-notify=exec /usr/bin/pw-play /tmp/failure.wav\nbackground-color=#684148\n[category=failed]\non-notify=exec /usr/bin/pw-play /tmp/failure.wav\nbackground-color=#684148\n[category=warning]\non-notify=exec /usr/bin/pw-play /tmp/warning.wav\nbackground-color=#715536\n[mode=quiet]\non-notify=none\n";
    let parsed = Config::parse(fixture).unwrap();
    let modes = ["default", "quiet"];
    let n = Traits {
        app_name:"fixture", app_icon:"", category:"failed", desktop_entry:"",
        summary:"Job failed", body:"", urgency:"normal", anchor:"top-right", output:"",
        modes: &modes, requested_timeout_ms:-1, actionable:false, group_index:-1, hidden:false
    };
    let style=parsed.style_for(&n);
    assert_eq!(style["background-color"],"#684148");
    assert_eq!(style["on-notify"],"none");
    assert_eq!(style["width"],"440");
    assert_eq!(style["font"],"Atkinson Hyperlegible Next 13");
}

#[test]
fn notification_lifecycle_does_not_require_a_gui_or_database() {
    let mut store=NotificationStore::new(5);
    let id=store.push(Request { app_name:"fixture".into(), summary:"Hi".into(), ..Request::default() });
    assert_eq!(store.active().len(),1);
    assert!(store.close(id,CloseReason::Dismissed,true,true).is_some());
    assert_eq!(store.history().len(),1);
    assert_eq!(store.restore(),Some(id));
}

#[test]
fn posthornctl_usage_writes_stderr_with_mako_exit_semantics() {
    use std::process::Command;

    let binary=env!("CARGO_BIN_EXE_posthornctl");
    let no_args=Command::new(binary).output().unwrap();
    assert!(!no_args.status.success(),"bare invocation must exit nonzero");
    assert!(no_args.stdout.is_empty(),"usage must not contaminate stdout");
    assert!(String::from_utf8_lossy(&no_args.stderr).contains(
        "Usage: posthornctl <command> [options...]"
    ));

    for flag in ["help","-h","--help"] {
        let help=Command::new(binary).arg(flag).output().unwrap();
        assert!(help.status.success(),"{flag} should exit successfully");
        assert!(help.stdout.is_empty(),"{flag} must print usage to stderr");
        assert!(String::from_utf8_lossy(&help.stderr).contains(
            "Usage: posthornctl <command> [options...]"
        ));
    }
}
