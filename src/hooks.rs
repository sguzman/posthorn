//! Port of Mako's configured on-notify input binding.
//!
//! Unlike notification bodies, these commands come from a trusted local
//! configuration file. Never interpolate sender-controlled text into the shell.

use std::io;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotifyBinding {
    None,
    Exec(String),
    Dismiss { save_history: bool },
    DismissAll,
    DismissGroup,
    InvokeDefaultAction,
    InvokeAction(String),
}

impl NotifyBinding {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "none" => Ok(Self::None),
            "dismiss" => Ok(Self::Dismiss { save_history: true }),
            "dismiss --no-history" => Ok(Self::Dismiss { save_history: false }),
            "dismiss-all" => Ok(Self::DismissAll),
            "dismiss-group" => Ok(Self::DismissGroup),
            "invoke-default-action" => Ok(Self::InvokeDefaultAction),
            _ => {
                if let Some(cmd) = raw.strip_prefix("exec ") {
                    if cmd.trim().is_empty() { return Err("empty exec command".into()); }
                    return Ok(Self::Exec(cmd.into()));
                }
                if let Some(key) = raw.strip_prefix("invoke-action ") {
                    if key.is_empty() { return Err("empty action key".into()); }
                    return Ok(Self::InvokeAction(key.into()));
                }
                Err(format!("unknown Mako on-notify binding: {raw}"))
            }
        }
    }

    /// Only the configured Exec variant launches a subprocess; no notification
    /// fields are passed to the shell except the decimal ID, as with Mako.
    pub fn run_exec(&self, id: u32) -> io::Result<()> {
        if let Self::Exec(command) = self {
            // Mako's prepended shell assignment allows scripts to use $id.
            let script = format!("id=\"$1\"\n{command}");
            let mut child = Command::new("sh")
                .arg("-c").arg(script).arg("sh").arg(id.to_string())
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .spawn()?;
            // Reap detached notification hooks without blocking D-Bus.
            std::thread::spawn(move || { let _ = child.wait(); });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_mako_binding_values_exactly() {
        assert_eq!(NotifyBinding::parse("none").unwrap(), NotifyBinding::None);
        assert_eq!(NotifyBinding::parse("dismiss --no-history").unwrap(),
            NotifyBinding::Dismiss{save_history:false});
        assert_eq!(NotifyBinding::parse("invoke-default-action").unwrap(),
            NotifyBinding::InvokeDefaultAction);
        assert_eq!(NotifyBinding::parse("exec /usr/bin/pw-play sound.wav").unwrap(),
            NotifyBinding::Exec("/usr/bin/pw-play sound.wav".into()));
        assert!(NotifyBinding::parse("exec ").is_err());
        assert!(NotifyBinding::parse("magic").is_err());
    }

    #[test]
    fn none_is_a_safe_noop() {
        NotifyBinding::None.run_exec(8).unwrap();
    }
}
