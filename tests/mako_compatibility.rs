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
        surface_associated:true,output_known:true,modes: &modes, requested_timeout_ms:-1, actionable:false, group_index:-1, hidden:false
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
    assert!(no_args.stderr.ends_with(b"\\n"),"usage requires one final LF");
    assert!(!no_args.stderr.ends_with(b"\\n\\n"),
        "pinned Mako writes usage via fprintf without an extra blank line");

    // The parser's error diagnostics are user-visible CLI compatibility,
    // not generic Rust error formatting. These fail before session bus access.
    for (args,message) in [
        (vec!["dismiss","-a","-g"],"-a and -g cannot be used together"),
        (vec!["dismiss","-a","-n","3"],"-n cannot be used with -a or -g"),
        (vec!["menu"],"Missing menu command"),
        (vec!["mode","-s","-a","quiet"],"-a/-r/-t and -s cannot be used together"),
        (vec!["mode","quiet"],"positional arguments can only be used with -s"),
        (vec!["strange"],"Unknown command: strange"),
    ] {
        let output=Command::new(binary).args(&args)
            .env("DBUS_SESSION_BUS_ADDRESS","unix:path=/no/such/bus")
            .output().unwrap();
        assert!(!output.status.success(),"{args:?} unexpectedly accepted");
        assert!(output.stdout.is_empty(),"{args:?} leaked into stdout");
        assert_eq!(String::from_utf8_lossy(&output.stderr),
            format!("{message}\\n"),"makoctl diagnostic mismatch for {args:?}");
    }

    for flag in ["help","-h","--help"] {
        let help=Command::new(binary).arg(flag).output().unwrap();
        assert!(help.status.success(),"{flag} should exit successfully");
        assert!(help.stdout.is_empty(),"{flag} must print usage to stderr");
        assert!(String::from_utf8_lossy(&help.stderr).contains(
            "Usage: posthornctl <command> [options...]"
        ));
    }
}

#[test]
fn check_config_accepts_mako_cli_overrides_without_starting_a_daemon(){
    use std::process::Command;
    let temp=tempfile::tempdir().unwrap();
    let file=temp.path().join("mako.conf");
    std::fs::write(&file,
        "width=210\nmax-history=5\nsort=-time\n\
         [category=warning]\nwidth=340\n"
    ).unwrap();
    let binary=env!("CARGO_BIN_EXE_posthorn");
    let good=Command::new(binary).args([
        "--check-config",file.to_str().unwrap(),
        "--width=450","--font","Luciole 14",
        "--sort","+priority","--sort","-priority",
        "--max-history","2"
    ]).env("DBUS_SESSION_BUS_ADDRESS","unix:path=/no/such/bus")
      .env("WAYLAND_DISPLAY","not-a-real-display").output().unwrap();
    assert!(good.status.success(),"CLI option handling failed: {good:?}");
    let stdout=String::from_utf8_lossy(&good.stdout);
    assert!(stdout.contains("2 history slots; 5 CLI overrides"),
        "overrides did not apply after loading file: {stdout}");
    assert!(stdout.contains(&file.display().to_string()));

    // Mako's -c/--config selects the file before applying any later
    // root overrides, including the last of repeated -c options.
    let alternate=temp.path().join("other.conf");
    std::fs::write(&alternate,"max-history=8\n").unwrap();
    let explicit=Command::new(binary).args([
        "--check-config",file.to_str().unwrap(),
        "-c",file.to_str().unwrap(),
        "--config",alternate.to_str().unwrap()
    ]).output().unwrap();
    assert!(explicit.status.success(),"{explicit:?}");
    assert!(String::from_utf8_lossy(&explicit.stdout)
        .contains("8 history slots; 0 CLI overrides"));

    // GNU getopt_long("hc:") permits an attached short -c argument.
    // Mako treats this identically to the two-argv -c PATH spelling.
    let joined=format!("-c{}",alternate.display());
    let compact=Command::new(binary).args([
        "--check-config",file.to_str().unwrap(),joined.as_str(),
        "--width","480"
    ]).output().unwrap();
    assert!(compact.status.success(),"{compact:?}");
    assert!(String::from_utf8_lossy(&compact.stdout)
        .contains("8 history slots; 1 CLI overrides"),
        "joined -c must select the later config while retaining style args");

    // GNU getopt_long accepts abbreviated long-option names. Every
    // Mako style option shares flag=NULL, val=0 and required_argument,
    // so colliding prefixes resolve to the FIRST declaration, rather
    // than uniformly producing an ambiguous-option error.
    let prefix=Command::new(binary).args([
        "--check-config","--conf",alternate.to_str().unwrap(),
        "--wid=480","--heig","140","--hist=0",
        "--ico=0","--mar=5",
    ]).output().unwrap();
    assert!(prefix.status.success(),
        "source-style getopt abbreviations rejected: {prefix:?}");
    assert!(String::from_utf8_lossy(&prefix.stdout)
        .contains("8 history slots; 5 CLI overrides"));

    // "--ico" resolves to "icons" before icon-location/path; a numeric
    // Boolean succeeds. "--max" resolves first to max-icon-size, which
    // rejects zero even though max-history and max-visible permit zero.
    // "--he" is truly ambiguous because help and height have different
    // val/has_arg descriptors.
    for options in [
        vec!["--max=0"],
        vec!["--he=100"],
        vec!["--h=1"],
        vec!["--does-not-exist=1"],
    ] {
        let invalid=Command::new(binary).args(["--check-config",
            file.to_str().unwrap()]).args(&options).output().unwrap();
        assert!(!invalid.status.success(),
            "GNU getopt prefix discrepancy for {options:?}");
        assert!(invalid.stdout.is_empty());
    }

    for options in [
        vec!["--width","0"],
        vec!["--width","440\nmax-history=0"],
        vec!["--sort","bad"],
        vec!["--on-notify","dismiss"],
        vec!["--text-alignment","center"],
        vec!["--config"],
        vec!["--max-history","9223372036854775808"],
    ]{
        let output=Command::new(binary).args(["--check-config",
            file.to_str().unwrap()]).args(&options).output().unwrap();
        assert!(!output.status.success(),
            "invalid CLI option passed: {options:?}");
        assert!(output.stdout.is_empty(),
            "failure must not print a successful parse result");
    }
    // No implicit production Mako replacement is introduced by making
    // its syntax available in an explicitly safe validation command.
    let no_daemon=Command::new(binary).args(["--width","450"]).output().unwrap();
    assert!(!no_daemon.status.success(),
        "bare Mako options must not start a notification daemon");
    for options in [
        vec!["--width","0"],
        vec!["--on-notify","dismiss"],
        vec!["--default-timeout","-1"],
    ] {
        let output=Command::new(binary)
            .args(["--experimental-private-service",file.to_str().unwrap()])
            .args(&options)
            .env("DBUS_SESSION_BUS_ADDRESS","unix:path=/no/such/bus")
            .output().unwrap();
        assert!(!output.status.success(),
            "invalid private-service override must fail before claiming bus: {options:?}");
    }
    let extra=Command::new(binary).args([
        "--experimental-popup-preview",file.to_str().unwrap(),"--width","440",
    ]).output().unwrap();
    assert!(!extra.status.success(),
        "unrelated experimental command must not swallow extra options");

}
