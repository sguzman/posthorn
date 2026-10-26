use std::env;
use std::process::ExitCode;

#[tokio::main(flavor="current_thread")]
async fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    let command = match posthorn::ctl::parse_command(&args) {
        Ok(command) => command,
        Err(err) => {
            if err == posthorn::ctl::USAGE { eprint!("{err}"); }
            else { eprintln!("{err}"); }
            return ExitCode::FAILURE;
        }
    };
    match posthorn::ctl::execute(command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if let Some(mako)=err.downcast_ref::<posthorn::ctl::MakoCtlMenuError>(){
                eprintln!("{mako}");
            }else{
                eprintln!("posthornctl: {err}");
            }
            ExitCode::FAILURE
        }
    }
}
