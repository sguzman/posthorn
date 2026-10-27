use posthorn::config::Config;
use std::env;
use std::path::Path;
use std::process::ExitCode;

/// Parse Mako options before touching a bus. Also used by the explicitly
/// selected private-service CLI, never by the normal production startup.
fn parse_mako_cli_args(arguments:&[String])
    ->Result<(Option<String>,Vec<(String,String)>),String>
{
    parse_mako_cli_args_with_mode(arguments, env::var_os("POSIXLY_CORRECT").is_some())
}

// GNU getopt_long stops at the first non-option when POSIXLY_CORRECT is
// present, and otherwise permutes options following non-option operands.
// The explicit positional config path is a Posthorn development convenience;
// -c/--config always takes precedence over it.
fn parse_mako_cli_args_with_mode(arguments:&[String],stop_at_operand:bool)
    ->Result<(Option<String>,Vec<(String,String)>),String>
{
    let mut positional_path:Option<String>=None;
    let mut selected_config:Option<String>=None;
    let mut overrides:Vec<(String,String)>=Vec::new();
    let mut i=0;
    while i<arguments.len(){
        let token=arguments[i].as_str();
        // Pinned Mako calls GNU getopt_long(): a standalone "--"
        // terminates option scanning. Trailing operands and option-like
        // strings are not reinterpreted as config overrides.
        if token=="--" { break; }
        if token=="-c"||token=="--config" {
            i+=1;
            let value=arguments.get(i).ok_or_else(||
                format!("{token} requires a path"))?;
            if value.is_empty(){return Err("empty config path".into());}
            selected_config=Some(value.clone());
        }else if let Some(value)=token.strip_prefix("--config="){
            if value.is_empty(){return Err("empty config path".into());}
            selected_config=Some(value.into());
        }else if let Some(value)=token.strip_prefix("-c"){
            // GNU getopt_long("hc:") accepts -cPATH with the mandatory
            // argument attached to the short option, not only -c PATH.
            // The exact "-c" spelling is handled by the branch above.
            if value.is_empty(){return Err("empty config path".into());}
            selected_config=Some(value.into());
        }else if let Some(option)=token.strip_prefix("--"){
            let (provided,attached)=match option.split_once('='){
                Some((name,value))=>(name,Some(value)),
                None=>(option,None),
            };
            if provided.is_empty(){return Err("empty long option".into());}
            // GNU getopt_long accepts unique abbreviations, and selects
            // the FIRST compatible declaration when several declarations
            // have identical (flag, val, has_arg). All Mako style flags
            // have (NULL, 0, required_argument); preserve the exact
            // ordering in config.c rather than rejecting such prefixes.
            let matched=if provided=="help"||provided=="config"
                || posthorn::config::MAKO_CLI_STYLE_OPTIONS.contains(&provided){
                provided
            }else{
                let style=posthorn::config::MAKO_CLI_STYLE_OPTIONS.iter()
                    .copied().find(|name|name.starts_with(provided));
                let config="config".starts_with(provided);
                let help="help".starts_with(provided);
                if (config||help) && (style.is_some() || (config&&help)){
                    return Err(format!("ambiguous Mako option '--{provided}'"));
                }
                if config{"config"}
                else if help{"help"}
                else{style.ok_or_else(||
                    format!("unsupported Mako command-line option '--{provided}'"))?}
            };
            if matched=="help"{
                return Err("Mako --help is not a config override".into());
            }
            let value=if let Some(value)=attached{value.to_owned()}
                else{
                    i+=1;
                    arguments.get(i).ok_or_else(||
                        format!("--{provided} requires a value"))?.clone()
                };
            if matched=="config"{
                if value.is_empty(){return Err("empty config path".into());}
                selected_config=Some(value);
            }else{
                overrides.push((matched.into(),value));
            }
        }else if token == "-" || !token.starts_with('-'){
            // GNU getopt_long accepts every non-option operand, including
            // a lone "-". In the default permutation mode it continues to
            // parse later flags even after multiple operands. Only the
            // first operand is retained as the private entrypoint's
            // optional positional config path.
            if positional_path.is_none() { positional_path=Some(token.into()); }
            if stop_at_operand { break; }
        }else{
            return Err(format!("unexpected argument '{token}'"));
        }
        i+=1;
    }
    let path=selected_config.or(positional_path);
    Ok((path,overrides))
}

#[cfg(test)]
mod cli_terminator_tests {
    use super::parse_mako_cli_args;

    fn args(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|token| (*token).to_owned()).collect()
    }

    #[test]
    fn gnu_permutation_skips_multiple_operands_and_lone_dash() {
        let (path, overrides) = super::parse_mako_cli_args_with_mode(
            &args(&["-c", "/tmp/explicit.conf", "first", "second", "-",
                    "--width=205"]), false
        ).unwrap();
        assert_eq!(path.as_deref(), Some("/tmp/explicit.conf"));
        assert_eq!(overrides, vec![("width".into(), "205".into())]);
    }

    #[test]
    fn posixly_correct_treats_lone_dash_as_first_operand() {
        let (path, overrides) = super::parse_mako_cli_args_with_mode(
            &args(&["-c", "/tmp/explicit.conf", "-", "--width=777"]), true
        ).unwrap();
        assert_eq!(path.as_deref(), Some("/tmp/explicit.conf"));
        assert!(overrides.is_empty());
    }

    #[test]
    fn posixly_correct_stops_at_first_operand() {
        let (path, overrides) = super::parse_mako_cli_args_with_mode(
            &args(&["-c", "/tmp/pinned.conf", "--width=205", "ignored",
                    "--width=777"]), true
        ).unwrap();
        assert_eq!(path.as_deref(), Some("/tmp/pinned.conf"));
        assert_eq!(overrides, vec![("width".into(), "205".into())]);
    }

    #[test]
    fn gnu_permutation_accepts_options_after_operand_by_default() {
        let (path, overrides) = super::parse_mako_cli_args_with_mode(
            &args(&["-c", "/tmp/pinned.conf", "ignored", "--width=777"]), false
        ).unwrap();
        assert_eq!(path.as_deref(), Some("/tmp/pinned.conf"));
        assert_eq!(overrides, vec![("width".into(), "777".into())]);
    }

    #[test]
    fn double_dash_stops_options_and_ignores_trailing_operands() {
        let (path, overrides) = parse_mako_cli_args(
            &args(&["--width=321", "--", "--height=99", "unused.conf"])
        ).unwrap();
        assert_eq!(path, None);
        assert_eq!(overrides, vec![("width".into(), "321".into())]);
    }

    #[test]
    fn double_dash_preserves_explicit_config_and_prior_override() {
        let (path, overrides) = parse_mako_cli_args(
            &args(&["-c", "/tmp/pinned.conf", "--height=104", "--",
                    "--config=/tmp/ignored.conf", "--width=600"])
        ).unwrap();
        assert_eq!(path.as_deref(), Some("/tmp/pinned.conf"));
        assert_eq!(overrides, vec![("height".into(), "104".into())]);
    }

    #[test]
    fn required_option_argument_may_itself_be_two_dashes() {
        // GNU getopt_long consumes the next argv item as -c's required
        // argument; in that position "--" is not an option terminator.
        let (path, overrides) = parse_mako_cli_args(
            &args(&["-c", "--", "--width=312"])
        ).unwrap();
        assert_eq!(path.as_deref(), Some("--"));
        assert_eq!(overrides, vec![("width".into(), "312".into())]);
    }
}

/// Upstream Mako loads the chosen config, then replays style options in
/// original argv order. This validation mode claims no D-Bus name.
fn check_config_with_mako_cli(arguments:&[String])->Result<(Config,String,usize),String>{
    let (path,overrides)=parse_mako_cli_args(arguments)?;
    let (mut config,source)=if let Some(path)=path {
        let config=Config::from_path(Path::new(&path)).map_err(|e|e.to_string())?;
        (config,path)
    }else{
        let (config,path)=Config::from_mako_defaults().map_err(|e|e.to_string())?;
        (config,path.map(|p|p.display().to_string())
            .unwrap_or_else(||"Mako built-in defaults".into()))
    };
    // CLI options modify only the root style, so scoped config sections
    // retain their higher-priority matching behavior.
    for (key,value) in &overrides{
        config.apply_cli_override(key,value).map_err(|e|e.to_string())?;
    }
    Ok((config,source,overrides.len()))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let arguments=env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().is_some_and(|first|first=="--check-config"){
        return match check_config_with_mako_cli(&arguments[1..]){
            Ok((config,source,count))=>{
                println!("Parsed config from {source}: {} criteria rules; {} history slots; {} CLI overrides",
                    config.rules.len(),config.max_history,count);
                ExitCode::SUCCESS
            }
            Err(error)=>{
                eprintln!("posthorn --check-config: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if arguments.first().is_some_and(|first|first=="--experimental-private-service"){
        return match parse_mako_cli_args(&arguments[1..]){
            Ok((Some(path),overrides))=>{
                // This entrypoint owns *only* org.posthorn.Development.
                // Validation completes before its private D-Bus name is
                // requested; the standard name remains unavailable here.
                match posthorn::service::run_private_service_from_path_with_cli_overrides(
                    Path::new(&path),&overrides
                ).await {
                    Ok(())=>ExitCode::SUCCESS,
                    Err(error)=>{
                        eprintln!("Posthorn test D-Bus service failed: {error}");
                        ExitCode::FAILURE
                    }
                }
            }
            Ok((None,overrides))=>{
                match posthorn::service::run_private_service_from_defaults_with_cli_overrides(
                    &overrides
                ).await {
                    Ok(())=>ExitCode::SUCCESS,
                    Err(error)=>{eprintln!("Posthorn private service default config failed: {error}");ExitCode::FAILURE}
                }
            }
            Err(error)=>{
                eprintln!("posthorn private CLI: {error}");
                ExitCode::FAILURE
            }
        };
    }
    // Production-name ownership is never implicit: authorization and an
    // exact controller-pinned session bus are mandatory before any start.
    if arguments.first().is_some_and(|arg|arg=="--reversible-production-trial"){
        if env::var("POSTHORN_PRODUCTION_TRIAL_AUTHORIZED").ok().as_deref()!=Some("1"){
            eprintln!("Posthorn trial refused: explicit authorization required");
            return ExitCode::FAILURE;
        }
        if env::var("POSTHORN_HYPRLAND_QA").ok().as_deref()==Some("1"){
            eprintln!("Posthorn trial refused: private QA cannot own the public name");
            return ExitCode::FAILURE;
        }
        let bus=env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
        let pin=env::var("POSTHORN_PRODUCTION_TRIAL_EXPECTED_BUS_ADDRESS").unwrap_or_default();
        if bus.is_empty() || pin.is_empty() || bus!=pin{
            eprintln!("Posthorn trial refused: session bus does not match controller pin");
            return ExitCode::FAILURE;
        }
        if !env::var("POSTHORN_PRODUCTION_TRIAL_MAX_SECONDS").ok()
            .and_then(|value|posthorn::service::parse_production_trial_seconds(&value))
            .is_some(){
            eprintln!("Posthorn trial refused: bounded trial lifetime of 1-600 seconds required");
            return ExitCode::FAILURE;
        }
        return match parse_mako_cli_args(&arguments[1..]){
            Ok((Some(path),opts))=>match posthorn::desktop::
                run_standard_trial_desktop_from_path_with_cli_overrides(
                    Path::new(&path),&opts).await{
                Ok(())=>ExitCode::SUCCESS,
                Err(e)=>{eprintln!("Posthorn trial failed: {e}");ExitCode::FAILURE}
            },
            Ok((None,opts))=>match posthorn::desktop::
                run_standard_trial_desktop_from_defaults_with_cli_overrides(&opts).await{
                Ok(())=>ExitCode::SUCCESS,
                Err(e)=>{eprintln!("Posthorn trial defaults failed: {e}");ExitCode::FAILURE}
            },
            Err(e)=>{eprintln!("Posthorn trial arguments: {e}");ExitCode::FAILURE}
        };
    }
    // Both private Wayland presenters now exercise the same pinned Mako
    // startup and reload CLI override order as the headless service.
    // This is deliberately BEFORE the generic arg-count guard. Neither
    // private mode can acquire org.freedesktop.Notifications.
    if matches!(arguments.first().map(String::as_str),
        Some("--experimental-private-desktop"|
             "--experimental-private-multi-desktop")) {
        let multi=arguments[0]=="--experimental-private-multi-desktop";
        return match parse_mako_cli_args(&arguments[1..]){
            Ok((Some(path),overrides))=>{
                let result=if multi{
                    posthorn::desktop::run_private_multi_desktop_from_path_with_cli_overrides(
                        Path::new(&path),&overrides
                    ).await
                }else{
                    posthorn::desktop::run_private_desktop_from_path_with_cli_overrides(
                        Path::new(&path),&overrides
                    ).await
                };
                match result{
                    Ok(())=>ExitCode::SUCCESS,
                    Err(error)=>{
                        eprintln!("Posthorn private desktop failed: {error}");
                        ExitCode::FAILURE
                    }
                }
            }
            Ok((None,overrides))=>{
                let result=if multi{
                    posthorn::desktop::run_private_multi_desktop_from_defaults_with_cli_overrides(
                        &overrides
                    ).await
                }else{
                    posthorn::desktop::run_private_desktop_from_defaults_with_cli_overrides(
                        &overrides
                    ).await
                };
                match result{
                    Ok(())=>ExitCode::SUCCESS,
                    Err(error)=>{eprintln!("Posthorn private desktop default config failed: {error}");ExitCode::FAILURE}
                }
            }
            Err(error)=>{
                eprintln!("Posthorn private desktop CLI: {error}");
                ExitCode::FAILURE
            }
        };
    }
    // Other experimental modes still expect exactly their documented
    // argument count, and cannot silently absorb unrelated Mako flags.
    if arguments.len()>2{
        eprintln!("unexpected extra Posthorn arguments");
        return ExitCode::FAILURE;
    }
    let mut args=arguments.into_iter();
    match (args.next().as_deref(), args.next()) {
        (Some("--version"), None) => {
            println!("posthorn {} (Mako parity target {})", env!("CARGO_PKG_VERSION"), posthorn::MAKO_BASELINE);
            ExitCode::SUCCESS
        }
        (Some("--help") | Some("-h"), None) => {
            println!("Posthorn (development stage; NOT a desktop notification daemon)\n\
                     Usage:\n\
                     \tposthorn --check-config [PATH] [--MAKO-OPTION VALUE...]\n\
                     \tposthorn --experimental-private-service [PATH] [--MAKO-OPTION VALUE...]\n\
                     \tposthorn --experimental-private-desktop [PATH] [--MAKO-OPTION VALUE...]\n\
                     \tposthorn --experimental-private-multi-desktop [PATH] [--MAKO-OPTION VALUE...]\n\
                     \tposthorn --reversible-production-trial [PATH] [--MAKO-OPTION VALUE...] (explicitly authorized)\n\
                     \tposthorn --experimental-popup-preview PATH\n\
                     \tposthorn --version\n\
                     \tposthorn --help\n\
                     Only the explicit private-service command starts a headless D-Bus server.\n\
                     It uses org.posthorn.Development, NOT org.freedesktop.Notifications.");
            ExitCode::SUCCESS
        }
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
                surface_associated:true,output_known:true,modes: &modes, requested_timeout_ms: note.requested_timeout_ms,
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
        _ => {
            eprintln!("Posthorn is not ready to replace Mako. Use --help or --check-config PATH.");
            ExitCode::FAILURE
        }
    }
}
