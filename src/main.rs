use posthorn::config::Config;
use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("--version"), None) => {
            println!("posthorn {} (Mako parity target {})", env!("CARGO_PKG_VERSION"), posthorn::MAKO_BASELINE);
            ExitCode::SUCCESS
        }
        (Some("--help") | Some("-h"), None) => {
            println!("Posthorn (development stage; NOT a running notification daemon)\n\
                     Usage:\n\
                     \tposthorn --check-config PATH\n\
                     \tposthorn --version\n\
                     \tposthorn --help\n\
                     The Mako-compatible D-Bus and Wayland runtime is not yet implemented.");
            ExitCode::SUCCESS
        }
        (Some("--check-config"), Some(path)) => match Config::from_path(Path::new(&path)) {
            Ok(config) => {
                println!("Parsed config: {} criteria rules; {} history slots", config.rules.len(), config.max_history);
                ExitCode::SUCCESS
            }
            Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
        },
        _ => {
            eprintln!("Posthorn is not ready to replace Mako. Use --help or --check-config PATH.");
            ExitCode::FAILURE
        }
    }
}
