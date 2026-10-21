use posthorn::config::Config;
use std::env;
use std::path::Path;
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("--version"), None) => {
            println!("posthorn {} (Mako parity target {})", env!("CARGO_PKG_VERSION"), posthorn::MAKO_BASELINE);
            ExitCode::SUCCESS
        }
        (Some("--help") | Some("-h"), None) => {
            println!("Posthorn (development stage; NOT a desktop notification daemon)\n\
                     Usage:\n\
                     \tposthorn --check-config PATH\n\
                     \tposthorn --experimental-private-service PATH\n\
                     \tposthorn --experimental-popup-preview PATH\n\
                     \tposthorn --version\n\
                     \tposthorn --help\n\
                     Only the explicit private-service command starts a headless D-Bus server.\n\
                     It uses org.posthorn.Development, NOT org.freedesktop.Notifications.");
            ExitCode::SUCCESS
        }
        (Some("--check-config"), Some(path)) => match Config::from_path(Path::new(&path)) {
            Ok(config) => {
                println!("Parsed config: {} criteria rules; {} history slots", config.rules.len(), config.max_history);
                ExitCode::SUCCESS
            }
            Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
        },
        (Some("--experimental-popup-preview"), Some(path)) => {
            use posthorn::notification::{NotificationStore, Request, Urgency};
            use posthorn::config::Traits;
            use posthorn::wayland::{ScreenCorner, present_frame_once};
            use std::time::Duration;

            let config = match Config::from_path(Path::new(&path)) {
                Ok(config) => config,
                Err(error) => { eprintln!("{error}"); return ExitCode::FAILURE; }
            };
            let mut store = NotificationStore::new(1);
            let id = store.push(Request {
                app_name: "Posthorn preview".into(), category: "success".into(),
                summary: "Posthorn renderer".into(),
                body: "Wayland + Cairo/Pango sample".into(),
                urgency: Urgency::Normal,
                ..Request::default()
            });
            let note = store.active().iter().find(|n| n.id == id).unwrap();
            let modes = ["default"];
            let traits = Traits {
                app_name: &note.app_name, app_icon: &note.app_icon,
                category: &note.category, desktop_entry: &note.desktop_entry,
                summary: &note.summary, body: &note.body,
                urgency: "normal", anchor: "top-right", output: "",
                modes: &modes, requested_timeout_ms: note.requested_timeout_ms,
                actionable: false, group_index: -1, hidden: false,
            };
            let style = config.style_for(&traits);
            let result = posthorn::render::render_notification(note, &style)
                .map_err(|e| e.to_string())
                .and_then(|frame| present_frame_once(
                    &frame, ScreenCorner::TopRight, 10,
                    Duration::from_millis(1800)
                ).map_err(|e| e.to_string()));
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => { eprintln!("Experimental Wayland preview: {e}"); ExitCode::FAILURE }
            }
        }
        (Some("--experimental-private-service"), Some(path)) => {
            let config = match Config::from_path(Path::new(&path)) {
                Ok(config) => config,
                Err(error) => { eprintln!("{error}"); return ExitCode::FAILURE; }
            };
            match posthorn::service::run_private_service(config).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => { eprintln!("Posthorn test D-Bus service failed: {error}"); ExitCode::FAILURE }
            }
        }
        _ => {
            eprintln!("Posthorn is not ready to replace Mako. Use --help or --check-config PATH.");
            ExitCode::FAILURE
        }
    }
}
