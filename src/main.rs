use posthorn::config::Config;
use std::env;
use std::path::Path;
use std::process::ExitCode;

/// Parse Mako options before touching a bus. Also used by the explicitly
/// selected private-service CLI, never by the normal production startup.
fn parse_mako_cli_args(arguments:&[String])
    ->Result<(Option<String>,Vec<(String,String)>),String>
{
    let mut positional_path:Option<String>=None;
    let mut selected_config:Option<String>=None;
    let mut overrides:Vec<(String,String)>=Vec::new();
    let mut i=0;
    while i<arguments.len(){
        let token=arguments[i].as_str();
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
        }else if positional_path.is_none() && !token.starts_with('-'){
            positional_path=Some(token.into());
        }else{
            return Err(format!("unexpected argument '{token}'"));
        }
        i+=1;
    }
    let path=selected_config.or(positional_path);
    Ok((path,overrides))
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
            Ok((None,_))=>{
                eprintln!("posthorn --experimental-private-service requires a config path");
                ExitCode::FAILURE
            }
            Err(error)=>{
                eprintln!("posthorn private CLI: {error}");
                ExitCode::FAILURE
            }
        };
    }
    // Every other experimental mode expects exactly its documented path.
    // Do not silently swallow Mako CLI arguments on a different mode.
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
                     \tposthorn --experimental-private-service PATH [--MAKO-OPTION VALUE...]\n\
                     \tposthorn --experimental-private-desktop PATH\n\
                     \tposthorn --experimental-private-multi-desktop PATH\n\
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
        (Some("--experimental-private-desktop"), Some(path)) => {
            match posthorn::desktop::run_private_desktop_from_path(Path::new(&path)).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => { eprintln!("Posthorn private desktop failed: {err}"); ExitCode::FAILURE }
            }
        }
        (Some("--experimental-private-multi-desktop"), Some(path)) => {
            match posthorn::desktop::run_private_multi_desktop_from_path(
                Path::new(&path)
            ).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("Posthorn private multi-surface desktop failed: {err}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("Posthorn is not ready to replace Mako. Use --help or --check-config PATH.");
            ExitCode::FAILURE
        }
    }
}
