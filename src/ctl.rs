//! Partial Makoctl-compatible CLI, deliberately targeting the private
//! Posthorn development service while the desktop daemon is unfinished.
//! Future MVP must add menu, exact text formatting, real daemon selection and
//! full D-Bus error behavior before treating this as makoctl parity.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command as ProcessCommand, Stdio};
use zbus::{zvariant::OwnedValue, Connection, Proxy};

use crate::control::{CONTROL_INTERFACE, CONTROL_PATH};
use crate::service::PRIVATE_BUS_NAME;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Dismiss { id: u32, all: bool, group: bool, history: bool },
    Restore,
    Menu { id: u32, program: Vec<String> },
    Invoke { id: u32, action: String },
    List { history: bool, json: bool },
    Reload,
    Modes { edit: Vec<(char, String)> },
    Help,
}

pub fn parse_command(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "help" | "-h" | "--help" => Ok(Command::Help),
        "restore" if rest.is_empty() => Ok(Command::Restore),
        "reload" if rest.is_empty() => Ok(Command::Reload),
        "list" | "history" => {
            if rest.is_empty() || rest == ["-j"] {
                Ok(Command::List { history: command == "history", json: !rest.is_empty() })
            } else { Err("usage: posthornctl list|history [-j]".into()) }
        }
        "menu" => {
            let mut id=0;
            let mut i=0;
            while i<rest.len() {
                if rest[i]=="-n" {
                    i+=1;
                    id=rest.get(i).ok_or("missing notification ID")?
                        .parse().map_err(|_|"invalid notification ID")?;
                    i+=1;
                } else if rest[i]=="--" {
                    i+=1;
                    break;
                } else {
                    break;
                }
            }
            let program=rest[i..].to_vec();
            if program.is_empty(){return Err("missing menu command".into());}
            Ok(Command::Menu{id,program})
        }
        "dismiss" => {
            let mut id = 0u32;
            let (mut all, mut group, mut history) = (false, false, true);
            let mut i=0;
            while i<rest.len() {
                match rest[i].as_str() {
                    "-a" | "--all" => all=true,
                    "-g" | "--group" => group=true,
                    "-h" | "--no-history" => history=false,
                    "-n" => {
                        i += 1;
                        id = rest.get(i).ok_or("missing notification ID")?
                            .parse().map_err(|_| "invalid notification ID")?;
                    }
                    other => return Err(format!("unknown dismiss option {other}")),
                }
                i += 1;
            }
            if (all && group) || (id != 0 && (all || group)) {
                return Err("invalid combination of dismiss options".into());
            }
            Ok(Command::Dismiss { id, all, group, history })
        }
        "invoke" => {
            let mut id=0u32;
            let mut action="default".to_owned();
            let mut i=0;
            while i<rest.len() {
                if rest[i]=="-n" {
                    i += 1;
                    id = rest.get(i).ok_or("missing notification ID")?
                        .parse().map_err(|_| "invalid notification ID")?;
                } else if i == rest.len()-1 && action=="default" {
                    action=rest[i].clone();
                } else {
                    return Err("usage: posthornctl invoke [-n id] [action]".into());
                }
                i += 1;
            }
            Ok(Command::Invoke {id, action})
        }
        "mode" => {
            let mut edits=Vec::new();
            let mut i=0;
            while i<rest.len() {
                let flag=rest[i].as_str();
                if !["-a", "-r", "-t", "-s"].contains(&flag) {
                    return Err(format!("unknown mode option {flag}"));
                }
                let op=flag.as_bytes()[1] as char;
                i+=1;
                let mut count=0;
                while i<rest.len() && !rest[i].starts_with('-') {
                    edits.push((op,rest[i].clone()));
                    i+=1;
                    count+=1;
                }
                if count==0 { return Err(format!("missing value after {flag}")); }
            }
            Ok(Command::Modes {edit:edits})
        }
        other => Err(format!("unknown posthornctl command {other}")),
    }
}

pub fn update_modes(mut current: Vec<String>, edits: &[(char, String)]) -> Vec<String> {
    for (op,mode) in edits {
        match op {
            's' => { if !current.contains(mode) { current.push(mode.clone()); } },
            'a' => { if !current.contains(mode) { current.push(mode.clone()); } },
            'r' => current.retain(|value| value != mode),
            't' => {
                if current.contains(mode) { current.retain(|value| value != mode); }
                else { current.push(mode.clone()); }
            },
            _ => {},
        }
    }
    // -s is replacement, not additive; preserve argument order.
    if edits.iter().any(|(op,_)| *op == 's') {
        let mut selected=Vec::new();
        for (op,mode) in edits {
            if *op=='s' && !selected.contains(mode) {selected.push(mode.clone());}
        }
        selected
    } else { current }
}

/// Resolve a menu program's selected title to the Freedesktop action key.
fn selected_action_key(
    actions: &HashMap<String,String>, selected: &str,
) -> Option<String> {
    let mut entries:Vec<_>=actions.iter().collect();
    entries.sort_by(|a,b|a.0.cmp(b.0));
    entries.into_iter().find(|(_,title)|*title==selected)
        .map(|(key,_)|key.clone())
}

/// Pass labels via stdin and read a selection from a user-configured menu
/// application, without evaluating the labels in a shell.
fn choose_from_menu(
    program: &[String], actions: &HashMap<String,String>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if program.is_empty() || actions.is_empty() {
        return Err("menu command and actions are required".into());
    }
    let mut entries:Vec<_>=actions.iter().collect();
    entries.sort_by(|a,b|a.0.cmp(b.0));
    let mut child=ProcessCommand::new(&program[0]).args(&program[1..])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
    if let Some(mut input)=child.stdin.take() {
        for (_,label) in entries {
            writeln!(input,"{label}")?;
        }
    }
    let output=child.wait_with_output()?;
    if !output.status.success(){return Err(format!(
        "menu command exited with {}",output.status
    ).into());}
    let result=String::from_utf8(output.stdout)?;
    let selected=result.lines().next().ok_or("no action selected")?;
    selected_action_key(actions,selected)
        .ok_or_else(||"menu returned a title not present in notification actions".into())
}

fn value_string(map: &HashMap<String,OwnedValue>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| String::try_from(v.clone()).ok())
        .filter(|s| !s.is_empty())
}

fn json_entry(map: &HashMap<String,OwnedValue>) -> serde_json::Value {
    let urgency = map.get("urgency").and_then(|v| u8::try_from(v).ok());
    let urgency = match urgency { Some(0)=>Some("low"),Some(1)=>Some("normal"),
        Some(2)=>Some("critical"),_=>None };
    let actions: HashMap<String,String> = map.get("actions")
        .and_then(|v| HashMap::try_from(v.clone()).ok()).unwrap_or_default();
    serde_json::json!({
        "id": map.get("id").and_then(|v| u32::try_from(v).ok()),
        "app_name": value_string(map,"app-name"),
        "app_icon": value_string(map,"app-icon"),
        "category": value_string(map,"category"),
        "desktop_entry": value_string(map,"desktop-entry"),
        "summary": value_string(map,"summary"),
        "body": value_string(map,"body"),
        "urgency": urgency,
        "actions": actions
    })
}

pub async fn execute(command: Command) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if command == Command::Help {
        println!("posthornctl (development, private D-Bus only)\n          dismiss [-n id] [-a|-g] [-h] | restore | invoke [-n id] [action]\n          list [-j] | history [-j] | menu [-n id] -- program [args...] | mode [-s|-a|-r|-t mode] | reload");
        return Ok(());
    }
    let conn=Connection::session().await?;
    let proxy=Proxy::new(&conn,PRIVATE_BUS_NAME,CONTROL_PATH,CONTROL_INTERFACE).await?;
    match command {
        Command::Dismiss {id,all,group,history} => {
            let options: HashMap<String,OwnedValue> = [
                ("id".into(),OwnedValue::from(id)),
                ("all".into(),OwnedValue::from(all)),
                ("group".into(),OwnedValue::from(group)),
                ("history".into(),OwnedValue::from(history))
            ].into();
            let _: ()=proxy.call("DismissNotifications",&(options,)).await?;
        }
        Command::Restore => { let _: ()=proxy.call("RestoreNotification",&()).await?; }
        Command::Invoke {id,action} => {
            let _: ()=proxy.call("InvokeAction",&(id,action)).await?;
        }
        Command::List {history,json} => {
            let method=if history {"ListHistory"} else {"ListNotifications"};
            let records: Vec<HashMap<String,OwnedValue>>=proxy.call(method,&()).await?;
            if json {
                let values: Vec<_>=records.iter().map(json_entry).collect();
                println!("{}",serde_json::to_string_pretty(&values)?);
            } else {
                for r in &records {
                    let id=r.get("id").and_then(|v|u32::try_from(v).ok()).unwrap_or(0);
                    let name=value_string(r,"app-name").unwrap_or_default();
                    let summary=value_string(r,"summary").unwrap_or_default();
                    println!("{id}: {name}: {summary}");
                }
            }
        }
        Command::Menu {id,program} => {
            let records:Vec<HashMap<String,OwnedValue>>=
                proxy.call("ListNotifications",&()).await?;
            let note=records.iter().find(|record| {
                id==0 || record.get("id").and_then(|v|u32::try_from(v).ok())==Some(id)
            }).ok_or("notification not found")?;
            let notification_id=note.get("id")
                .and_then(|v|u32::try_from(v).ok()).ok_or("missing notification ID")?;
            let actions:HashMap<String,String>=note.get("actions")
                .and_then(|v|HashMap::try_from(v.clone()).ok())
                .ok_or("notification has no actions")?;
            let action=choose_from_menu(&program,&actions)?;
            let _: ()=proxy.call("InvokeAction",&(notification_id,action)).await?;
        }
        Command::Reload => {let _: ()=proxy.call("Reload",&()).await?;}
        Command::Modes {edit} => {
            let modes: Vec<String>=proxy.call("ListModes",&()).await?;
            if edit.is_empty() {
                for mode in modes {println!("{mode}");}
            } else {
                let replacement=update_modes(modes,&edit);
                let _: ()=proxy.call("SetModes",&(replacement,)).await?;
            }
        }
        Command::Help => unreachable!()
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(args:&[&str])->Vec<String> {args.iter().map(|s|s.to_string()).collect()}
    #[test]
    fn parses_dismiss_without_conflicting_flags() {
        assert_eq!(parse_command(&v(&["dismiss","-n","42","-h"])).unwrap(),
            Command::Dismiss{id:42,all:false,group:false,history:false});
        assert!(parse_command(&v(&["dismiss","-a","-g"])).is_err());
        assert!(parse_command(&v(&["dismiss","-n","not-numeric"])).is_err());
    }
    #[test]
    fn mode_operations_are_ordered_and_deduplicated() {
        let edits=vec![('a',"quiet".into()),('t',"default".into())];
        assert_eq!(update_modes(vec!["default".into()],&edits),vec!["quiet"]);
        let selected=vec![('s',"quiet".into()),('s',"custom".into())];
        assert_eq!(update_modes(vec!["default".into()],&selected),vec!["quiet","custom"]);
    }
    #[test]
    fn menu_parse_requires_command_and_supports_explicit_notification_id() {
        assert_eq!(parse_command(&v(&["menu","-n","34","--","cat"])).unwrap(),
            Command::Menu{id:34,program:vec!["cat".into()]});
        assert!(parse_command(&v(&["menu","-n","34"])).is_err());
        assert!(parse_command(&v(&["menu"])).is_err());
    }

    #[test]
    fn selecting_menu_title_recovers_correct_action_key() {
        let actions:HashMap<_,_>=[
            ("default".into(),"Open".into()),
            ("dismiss".into(),"Ignore".into())
        ].into();
        assert_eq!(selected_action_key(&actions,"Open"),Some("default".into()));
        assert!(selected_action_key(&actions,"no such title").is_none());
        let chosen=choose_from_menu(&["cat".into()],&actions).unwrap();
        assert_eq!(chosen,"default");
    }

    #[test]
    fn accepts_mako_list_and_default_invoke() {
        assert_eq!(parse_command(&v(&["history","-j"])).unwrap(),
            Command::List{history:true,json:true});
        assert_eq!(parse_command(&v(&["invoke"])).unwrap(),
            Command::Invoke{id:0,action:"default".into()});
    }
}
