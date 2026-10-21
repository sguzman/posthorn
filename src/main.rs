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
