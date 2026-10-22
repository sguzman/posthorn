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
                } else if let Some(value)=rest[i].strip_prefix("-n") {
                    if value.is_empty() { break; }
                    id=value.parse().map_err(|_|"invalid notification ID")?;
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
                    other if other.starts_with("-n") => {
                        id = other[2..].parse().map_err(|_| "invalid notification ID")?;
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
                } else if let Some(value)=rest[i].strip_prefix("-n") {
                    id = value.parse().map_err(|_| "invalid notification ID")?;
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
            // Upstream makoctl uses getopt("a:r:t:s"): -a/-r/-t take
            // exactly one argument; -s is a flag whose replacement modes
            // are positional arguments, and may be an empty list.
            let mut edits=Vec::new();
            let mut positional=Vec::new();
            let mut set=false;
            let mut i=0;
            while i<rest.len() {
                match rest[i].as_str() {
                    "--"=>{
                        positional.extend(rest[i+1..].iter().cloned());
                        break;
                    }
                    "-s"=>{set=true;i+=1;}
                    "-a"|"-r"|"-t"=>{
                        let flag=rest[i].as_str();
                        let op=flag.as_bytes()[1] as char;
                        i+=1;
                        let mode=rest.get(i).ok_or_else(||
                            format!("missing value after {flag}"))?;
                        if mode.starts_with('-'){
                            return Err(format!("missing value after {flag}"));
                        }
                        edits.push((op,mode.clone()));
                        i+=1;
                    }
                    other if other.starts_with('-')=>
                        return Err(format!("unknown mode option {other}")),
                    value=>{positional.push(value.to_owned());i+=1;}
                }
            }
            if set && !edits.is_empty() {
                return Err("mode -s cannot be combined with -a/-r/-t".into());
            }
            if set {
                // A marker is retained even if no positional mode follows.
                edits.push(('s',String::new()));
                edits.extend(positional.into_iter().map(|m|('S',m)));
            } else if !positional.is_empty() {
                return Err("positional mode arguments require -s".into());
            }
            Ok(Command::Modes {edit:edits})
        }
        other => Err(format!("unknown posthornctl command {other}")),
    }
}

/// The control client prints the resulting modes even after modifying them,
/// matching makoctl v1.11.0 rather than only printing on bare `mode`.
pub fn modes_text(modes:&[String])->String {
    modes.iter().map(|mode|format!("{mode}\n")).collect()
}

pub fn update_modes(mut current: Vec<String>, edits: &[(char, String)]) -> Vec<String> {
    // Mako's -s replaces *all* modes, including an explicit empty list.
    if edits.iter().any(|(op,_)|*op=='s'){
        let mut selected:Vec<String>=edits.iter()
            .filter(|(op,value)|*op=='s'&&!value.is_empty())
            .map(|(_,value)|value.clone()).collect();
        selected.extend(edits.iter().filter(|(op,_)|*op=='S')
            .map(|(_,value)|value.clone()));
        return selected;
    }
    for (op,mode) in edits {
        let index=current.iter().position(|value|value==mode);
        match (op,index){
            // Upstream makoctl permits duplicate additions.
            ('a',_)=>current.push(mode.clone()),
            ('r',Some(index))=>{current.swap_remove(index);},
            ('t',Some(index))=>{current.swap_remove(index);},
            ('t',None)=>current.push(mode.clone()),
            _=>{}
        }
    }
    current
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

fn json_quoted(raw:&str)->String {
    serde_json::Value::String(raw.to_owned()).to_string()
}

fn json_field(map:&HashMap<String,OwnedValue>,key:&str)->String{
    value_string(map,key).as_deref().map(json_quoted)
        .unwrap_or_else(||"null".into())
}

/// Mako v1.11 prints a fixed key order and formats the actions object inline.
/// JSON consumers need not rely on key order, but scripts often diff makoctl
/// output as text, so preserve the upstream CLI representation as well.
pub fn notification_json_text(map:&HashMap<String,OwnedValue>)->String{
    let id=map.get("id").and_then(|v|u32::try_from(v).ok()).unwrap_or(0);
    let mut result=format!("  {{\n    \"id\": {id},\n");
    for (wire,json_key) in [
        ("app-name","app_name"),("app-icon","app_icon"),
        ("category","category"),("desktop-entry","desktop_entry"),
        ("summary","summary"),("body","body"),
    ]{
        result.push_str(&format!("    \"{json_key}\": {},\n",json_field(map,wire)));
    }
    let urgency=map.get("urgency").and_then(|v|u8::try_from(v).ok());
    let urgency=match urgency{
        Some(0)=>Some("low"),Some(1)=>Some("normal"),
        Some(2)=>Some("critical"),_=>None,
    };
    result.push_str(&format!("    \"urgency\": {},\n",
        urgency.map(json_quoted).unwrap_or_else(||"null".into())));
    result.push_str("    \"actions\": {");
    let actions:HashMap<String,String>=map.get("actions")
        .and_then(|v|HashMap::try_from(v.clone()).ok()).unwrap_or_default();
    let mut actions:Vec<_>=actions.into_iter().collect();
    actions.sort_by(|a,b|a.0.cmp(&b.0));
    for (index,(key,label)) in actions.iter().enumerate(){
        if index>0{result.push_str(", ");}
        result.push_str(&json_quoted(key));
        result.push_str(": ");
        result.push_str(&json_quoted(label));
    }
    result.push_str("}\n  }");
    result
}

pub fn notification_list_json_text(records:&[HashMap<String,OwnedValue>])->String{
    let mut out=String::from("[");
    for (index,record) in records.iter().enumerate(){
        if index>0{out.push(',');}
        out.push('\n');
        out.push_str(&notification_json_text(record));
    }
    out.push_str("\n]\n");
    out
}

/// Match makoctl's human-readable list/history layout. The app icon and
/// message body are intentionally omitted, as in upstream v1.11.0.
pub fn notification_text(map:&HashMap<String,OwnedValue>)->String{
    let id=map.get("id").and_then(|v|u32::try_from(v).ok()).unwrap_or(0);
    let summary=value_string(map,"summary").unwrap_or_default();
    let mut result=format!("Notification {id}:{title}\n",
        title=if summary.is_empty(){String::new()}else{format!(" {summary}")});
    for (field,title) in [
        ("app-name","App name"),
        ("category","Category"),
        ("desktop-entry","Desktop entry"),
    ]{
        if let Some(value)=value_string(map,field){
            result.push_str(&format!("  {title}: {value}\n"));
        }
    }
    let urgency=map.get("urgency").and_then(|v|u8::try_from(v).ok());
    let urgency=match urgency {
        Some(0)=>Some("low"),Some(1)=>Some("normal"),
        Some(2)=>Some("critical"),_=>None
    };
    if let Some(urgency)=urgency {
        result.push_str(&format!("  Urgency: {urgency}\n"));
    }
    if let Some(actions)=map.get("actions")
        .and_then(|v|HashMap::<String,String>::try_from(v.clone()).ok())
        .filter(|actions|!actions.is_empty())
    {
        result.push_str("  Actions:\n");
        let mut pairs:Vec<_>=actions.into_iter().collect();
        pairs.sort_by(|a,b|a.0.cmp(&b.0));
        for (key,label) in pairs{
            result.push_str(&format!("    {key}: {label}\n"));
        }
    }
    result
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
                print!("{}",notification_list_json_text(&records));
            } else {
                for record in &records{
                    print!("{}",notification_text(record));
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
                print!("{}",modes_text(&modes));
            } else {
                let replacement=update_modes(modes,&edit);
                let _: ()=proxy.call("SetModes",&(replacement,)).await?;
                print!("{}",modes_text(&replacement));
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
    fn mako_json_listing_preserves_field_order_nulls_and_inline_actions(){
        use zbus::zvariant::Value;
        let mut fields=HashMap::new();
        fields.insert("id".into(),OwnedValue::from(17u32));
        fields.insert("summary".into(),OwnedValue::try_from(
            Value::from("Quoted \"text\" & newline\n")).unwrap());
        fields.insert("urgency".into(),OwnedValue::from(2u8));
        fields.insert("actions".into(),OwnedValue::from(
            HashMap::from([("open".to_owned(),"Open".to_owned())])
        ));
        let rendered=notification_list_json_text(&[fields]);
        assert!(rendered.starts_with("[\n  {\n    \"id\": 17,\n"));
        assert!(rendered.contains("    \"app_name\": null,\n"));
        assert!(rendered.contains("    \"urgency\": \"critical\",\n"));
        assert!(rendered.contains("    \"actions\": {\"open\": \"Open\"}\n  }"));
        assert!(rendered.contains("Quoted"));
        assert_eq!(notification_list_json_text(&[]),"[\n]\n");
        let parsed:serde_json::Value=serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed[0]["id"],17);
        assert_eq!(parsed[0]["summary"],"Quoted \"text\" & newline\n");
    }

    #[test]
    fn human_readable_listing_follows_upstream_makoctl_structure(){
        use zbus::zvariant::Value;
        let mut fields=HashMap::new();
        fields.insert("id".into(),OwnedValue::from(7u32));
        for (key,value) in [
            ("summary","Task completed"),
            ("app-name","Backup"),
            ("category","success"),
            ("desktop-entry","backup.desktop"),
        ]{
            fields.insert(key.into(),OwnedValue::try_from(Value::from(value)).unwrap());
        }
        fields.insert("urgency".into(),OwnedValue::from(1u8));
        fields.insert("actions".into(),OwnedValue::from(
            HashMap::from([("default".to_string(),"Open".to_string())])
        ));
        assert_eq!(notification_text(&fields),
            "Notification 7: Task completed\n  App name: Backup\n  Category: success\n  Desktop entry: backup.desktop\n  Urgency: normal\n  Actions:\n    default: Open\n");
        let mut fields=HashMap::new();
        fields.insert("id".into(),OwnedValue::from(9u32));
        assert_eq!(notification_text(&fields),"Notification 9:\n");
        fields.insert("actions".into(),OwnedValue::from(HashMap::<String,String>::new()));
        assert_eq!(notification_text(&fields),"Notification 9:\n");
    }

    #[test]
    fn parses_dismiss_without_conflicting_flags() {
        assert_eq!(parse_command(&v(&["dismiss","-n","42","-h"])).unwrap(),
            Command::Dismiss{id:42,all:false,group:false,history:false});
        assert!(parse_command(&v(&["dismiss","-a","-g"])).is_err());
        assert!(parse_command(&v(&["dismiss","-n","not-numeric"])).is_err());
        assert_eq!(parse_command(&v(&["dismiss","-n42","-h"])).unwrap(),
            Command::Dismiss{id:42,all:false,group:false,history:false});
        assert!(parse_command(&v(&["dismiss","-nINVALID"])).is_err());
    }
    #[test]
    fn modes_match_makoctl_getopt_and_replacement_semantics() {
        let edits=vec![('a',"quiet".into()),('t',"default".into())];
        assert_eq!(update_modes(vec!["default".into()],&edits),vec!["quiet"]);
        assert_eq!(parse_command(&v(&["mode","-s","quiet","custom"])).unwrap(),
            Command::Modes{edit:vec![
                ('s',"".into()),('S',"quiet".into()),('S',"custom".into())
            ]});
        assert_eq!(update_modes(vec!["default".into()],&[
            ('s',"".into()),('S',"quiet".into()),('S',"custom".into())
        ]),vec!["quiet","custom"]);
        assert_eq!(update_modes(vec!["default".into()],&[('s',"".into())]),
            Vec::<String>::new());
        assert_eq!(parse_command(&v(&["mode","-s"])).unwrap(),
            Command::Modes{edit:vec![('s',"".into())]});
        assert!(parse_command(&v(&["mode","quiet"])).is_err());
        assert!(parse_command(&v(&["mode","-a"])).is_err());
        assert!(parse_command(&v(&["mode","-a","-t"])).is_err());
        assert!(parse_command(&v(&["mode","-s","-a","quiet"])).is_err());
        assert!(parse_command(&v(&["mode","-r","default","-s","quiet"])).is_err());
        assert!(parse_command(&v(&["mode","-t","quiet","-s"])).is_err());
        assert_eq!(parse_command(&v(&["mode","-s","--","-quiet"])).unwrap(),
            Command::Modes{edit:vec![('s',"".into()),('S',"-quiet".into())]});
        assert_eq!(parse_command(&v(&["mode","-a","quiet","-t","default"])).unwrap(),
            Command::Modes{edit:vec![('a',"quiet".into()),('t',"default".into())]});
        assert_eq!(update_modes(vec![],&[('a',"quiet".into()),('a',"quiet".into())]),
            vec!["quiet","quiet"]);
    }
    #[test]
    fn mode_stdout_matches_upstream_before_and_after_edits() {
        let original=vec!["default".to_string()];
        assert_eq!(modes_text(&original),"default\n");
        let after=update_modes(original,&[('a',"quiet".into()),('a',"quiet".into())]);
        // makoctl prints the local prospective list, even if SetModes itself
        // deduplicates values on the server.
        assert_eq!(modes_text(&after),"default\nquiet\nquiet\n");
        assert_eq!(modes_text(&update_modes(after,&[('s',String::new())])),"");
    }

    #[test]
    fn menu_parse_requires_command_and_supports_explicit_notification_id() {
        assert_eq!(parse_command(&v(&["menu","-n","34","--","cat"])).unwrap(),
            Command::Menu{id:34,program:vec!["cat".into()]});
        assert!(parse_command(&v(&["menu","-n","34"])).is_err());
        assert!(parse_command(&v(&["menu"])).is_err());
        assert_eq!(parse_command(&v(&["menu","-n34","cat"])).unwrap(),
            Command::Menu{id:34,program:vec!["cat".into()]});
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
        assert_eq!(parse_command(&v(&["invoke","-n34","default"])).unwrap(),
            Command::Invoke{id:34,action:"default".into()});
    }
}
