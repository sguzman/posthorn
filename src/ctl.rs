//! Partial Makoctl-compatible CLI, deliberately targeting the private
//! Posthorn development service while the desktop daemon is unfinished.
//! Future MVP must add menu, exact text formatting, real daemon selection and
//! full D-Bus error behavior before treating this as makoctl parity.

use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::process::{Command as ProcessCommand, Stdio};
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use zbus::{zvariant::{DeserializeValue, OwnedValue, Value}, Connection, Proxy};

use crate::control::{CONTROL_INTERFACE, CONTROL_PATH};
use crate::service::PRIVATE_BUS_NAME;

/// CLI usage mirrors makoctl 1.11, but Posthorn still targets a private
/// development D-Bus service; it is not a production daemon replacement.
pub const USAGE: &str = r#"Usage: posthornctl <command> [options...]

Commands:
  dismiss [-n id]                Dismiss the notification with the
                                 given id, or the last notification
                                 if none is given
          [-a|--all]             Dismiss all notifications
          [-g|--group]           Dismiss all the notifications
                                 in the last notification's group
          [-h|--no-history]      Dismiss w/o adding to history
  restore                        Restore the most recently expired
                                 notification from the history buffer
  invoke [-n id] [action]        Invoke an action on the notification
                                 with the given id, or the last
                                 notification if none is given
  menu [-n id] <prog> [arg ...]  Use <prog> [args ...] to select one
                                 action to be invoked on the notification
                                 with the given id, or the last
                                 notification if none is given
  list [-j]                      List notifications
  history [-j]                   List history
  reload                         Reload the configuration file
  mode                           List modes
  mode [-a mode]... [-r mode]... Add/remove modes
  mode [-t mode]...              Toggle modes (add if not present, remove if present)
  mode -s mode...                Set modes
  help                           Show this help

Development builds connect only to org.posthorn.Development.
"#;

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

/// Match makoctl's strtoumax(base 10) notification ID parsing.
/// It permits leading C-locale whitespace, a '+' sign, and -0. A
/// completely empty argument is parsed as zero by upstream strtoumax.
fn parse_mako_id(raw:&str)->Result<u32,&'static str>{
    if raw.is_empty(){return Ok(0);}
    // strtoumax skips C isspace, including vertical tab (0x0B).
    let digits=raw.trim_start_matches([' ','\t','\n','\r','\x0b','\x0c']);
    // strtoumax permits a minus sign, then negates the parsed unsigned
    // magnitude. Every spelling of negative zero therefore yields ID zero.
    if digits.strip_prefix('-').is_some_and(|number|
        !number.is_empty() && number.bytes().all(|ch|ch==b'0')
    ){return Ok(0);}
    digits.parse().map_err(|_|"invalid notification ID")
}

pub fn parse_command(args: &[String]) -> Result<Command, String> {
    let Some((command, rest)) = args.split_first() else {
        // makoctl with no command prints usage on stderr and exits nonzero.
        return Err(USAGE.into());
    };
    match command.as_str() {
        "help" | "-h" | "--help" => Ok(Command::Help),
        // Mako invokes these two methods without parsing any options.
        "restore" => Ok(Command::Restore),
        "reload" => Ok(Command::Reload),
        "list" | "history" => {
            // getopt("j") permutes options past ignored positional arguments.
            // Repeated and clustered -j switches enable JSON. Anything after
            // "--" is positional, including text that begins with '-'.
            let mut json=false;
            let mut options=true;
            for arg in rest {
                if !options { continue; }
                if arg=="--" {
                    options=false;
                } else if arg.starts_with('-') && arg.len()>1 {
                    if !arg[1..].bytes().all(|b|b==b'j') {
                        return Err("usage: posthornctl list|history [-j]".into());
                    }
                    json=true;
                }
            }
            Ok(Command::List {history:command=="history",json})
        }
        "menu" => {
            // Mako uses GNU getopt("n:"), which permutes options ahead of
            // positional program arguments until "--". -n after the
            // program name still selects the notification.
            let mut id=0u32;
            let mut program=Vec::new();
            let mut options=true;
            let mut i=0;
            while i<rest.len() {
                let arg=rest[i].as_str();
                if options && arg=="--" {
                    options=false;
                } else if options && arg=="-n" {
                    i+=1;
                    id=parse_mako_id(rest.get(i).ok_or("missing notification ID")?)?;
                } else if options && arg.len()>2 && arg.starts_with("-n") {
                    id=parse_mako_id(&arg[2..])?;
                } else if options && arg.starts_with('-') && arg.len()>1 {
                    return Err(format!("unknown menu option {arg}"));
                } else {
                    program.push(rest[i].clone());
                }
                i+=1;
            }
            if program.is_empty(){return Err("missing menu command".into());}
            Ok(Command::Menu{id,program})
        }
        "dismiss" => {
            // Upstream uses GNU getopt_long("aghn:"). It supports clustered
            // short flags, attached -n IDs, --, and option permutation.
            let mut id=0u32;
            let (mut all,mut group,mut history)=(false,false,true);
            let mut options=true;
            let mut i=0;
            while i<rest.len(){
                let arg=rest[i].as_str();
                if options && arg=="--" {
                    options=false;
                }else if options && arg.starts_with("--"){
                    // GNU getopt_long accepts unique abbreviations of
                    // --all, --group and --no-history (e.g. --gr, --no).
                    let name=&arg[2..];
                    if !name.is_empty() && "all".starts_with(name) {
                        all=true;
                    } else if !name.is_empty() && "group".starts_with(name) {
                        group=true;
                    } else if !name.is_empty() && "no-history".starts_with(name) {
                        history=false;
                    } else {
                        return Err(format!("unknown dismiss option {arg}"));
                    }
                }else if options && arg.starts_with('-') && arg.len()>1{
                    let mut chars=arg[1..].chars().peekable();
                    while let Some(option)=chars.next(){
                        match option{
                            'a'=>all=true,'g'=>group=true,'h'=>history=false,
                            'n'=>{
                                let attached:String=chars.collect();
                                let value=if attached.is_empty(){
                                    i+=1;
                                    rest.get(i).ok_or("missing notification ID")?.as_str()
                                }else{attached.as_str()};
                                id=parse_mako_id(value)?;
                                break;
                            }
                            _=>return Err(format!("unknown dismiss option -{option}")),
                        }
                    }
                }
                // GNU getopt ignores extra non-option arguments here.
                i+=1;
            }
            if (all && group) || (id != 0 && (all || group)) {
                return Err("invalid combination of dismiss options".into());
            }
            Ok(Command::Dismiss { id, all, group, history })
        }
        "invoke" => {
            // GNU getopt("n:") permits -n before or after the positional
            // action and honors --. Upstream makoctl chooses the first
            // remaining positional argument, ignoring later ones.
            let mut id=0u32;
            let mut action=None;
            let mut options=true;
            let mut i=0;
            while i<rest.len() {
                let arg=rest[i].as_str();
                if options && arg=="--" {
                    options=false;
                } else if options && arg=="-n" {
                    i+=1;
                    id=parse_mako_id(rest.get(i).ok_or("missing notification ID")?)?;
                } else if options && arg.starts_with("-n") {
                    id=parse_mako_id(&arg[2..])?;
                } else if options && arg.starts_with('-') && arg.len()>1 {
                    return Err(format!("unknown invoke option {arg}"));
                } else if action.is_none() {
                    action=Some(arg.to_owned());
                }
                i+=1;
            }
            Ok(Command::Invoke{id,action:action.unwrap_or_else(||"default".into())})
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
                        // GNU getopt accepts a required option argument even
                        // when its first character is '-'. Do not mistake a
                        // valid mode name for another flag.
                        edits.push((op,mode.clone()));
                        i+=1;
                    }
                    other if other.len()>2 && other.starts_with('-')
                        && other[1..].bytes().all(|ch|ch==b's')=>{
                        // getopt("a:r:t:s") accepts clustered -ss and -sss
                        // as repeated no-argument -s options.
                        set=true;
                        i+=1;
                    }
                    other if other.len()>2
                        && other.starts_with('-')
                        && matches!(other.as_bytes()[1],b'a'|b'r'|b't')=>
                    {
                        // getopt("a:r:t:s") permits compact -aquiet,
                        // -rquiet and -tquiet forms as well.
                        let op=other.as_bytes()[1] as char;
                        edits.push((op,other[2..].to_owned()));
                        i+=1;
                    }
                    other if other.starts_with('-') && other.len()>1=>
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


#[derive(Debug,zbus::zvariant::Type)]
#[zvariant(signature = "a{ss}")]
struct OrderedActionPairs(Vec<(String,String)>);

impl<'de> Deserialize<'de> for OrderedActionPairs {
    fn deserialize<D:Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
        struct Entries;
        impl<'de> Visitor<'de> for Entries {
            type Value=OrderedActionPairs;
            fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result{
                write!(f,"ordered D-Bus a{{ss}} action entries")
            }
            fn visit_map<A:MapAccess<'de>>(self,mut map:A)->Result<Self::Value,A::Error>{
                let mut pairs=Vec::new();
                while let Some((key,label))=map.next_entry::<String,String>()? {
                    pairs.push((key,label));
                }
                Ok(OrderedActionPairs(pairs))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

#[derive(Debug,zbus::zvariant::Type)]
#[zvariant(signature = "a{sv}")]
struct OrderedControlRecord {
    fields:HashMap<String,OwnedValue>,
    actions:Vec<(String,String)>,
}

impl<'de> Deserialize<'de> for OrderedControlRecord {
    fn deserialize<D:Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
        struct Record;
        impl<'de> Visitor<'de> for Record {
            type Value=OrderedControlRecord;
            fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result{
                write!(f,"Mako a{{sv}} control record with ordered actions")
            }
            fn visit_map<A:MapAccess<'de>>(self,mut map:A)->Result<Self::Value,A::Error>{
                let mut fields=HashMap::new();
                let mut actions=Vec::new();
                while let Some(key)=map.next_key::<String>()?{
                    if key=="actions" {
                        let value:DeserializeValue<'de,OrderedActionPairs>=
                            map.next_value()?;
                        actions=value.0.0;
                    }else{
                        fields.insert(key,map.next_value()?);
                    }
                }
                Ok(OrderedControlRecord{fields,actions})
            }
        }
        deserializer.deserialize_map(Record)
    }
}

/// Read a{ss} entries deterministically without converting to a HashMap.
/// The current zvariant representation sorts keys and collapses duplicates.
/// Mako's original action-list order remains an explicitly tracked parity gap.
fn action_pairs(map:&HashMap<String,OwnedValue>)->Vec<(String,String)>{
    let Some(value)=map.get("actions") else{return Vec::new()};
    let Value::Dict(dict)=&**value else{return Vec::new()};
    dict.iter().filter_map(|(key,label)|{
        let (Value::Str(key),Value::Str(label))=(key,label) else{return None};
        Some((key.as_str().to_owned(),label.as_str().to_owned()))
    }).collect()
}

/// Resolve a selected menu label against the first matching wire entry.
/// The exact Mako list order is not yet preserved by zvariant::Dict.
fn selected_action_key(
    actions:&[(String,String)],selected:&str,
)->Option<String>{
    actions.iter().find(|(_,title)|title==selected)
        .map(|(key,_)|key.clone())
}

/// makoctl getline() discards only a terminal LF from the first line.
/// Rust str::lines() also removes a preceding CR, which can spuriously
/// select an action that upstream Mako would reject.
fn mako_menu_selected_line(output:&str)->Option<&str>{
    if output.is_empty(){return None;}
    Some(output.split_once('\n').map_or(output,|(first,_)|first))
}

/// Pass labels via stdin and read a selection from a user-configured menu
/// application, without evaluating the labels in a shell.
fn choose_from_menu(
    program:&[String],actions:&[(String,String)],
)->Result<String,Box<dyn std::error::Error+Send+Sync>>{
    if program.is_empty()||actions.is_empty(){
        return Err("menu command and actions are required".into());
    }
    let mut child=ProcessCommand::new(&program[0]).args(&program[1..])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
    if let Some(mut input)=child.stdin.take(){
        for (_,label) in actions{
            writeln!(input,"{label}")?;
        }
    }
    let output=child.wait_with_output()?;
    if !output.status.success(){return Err(format!(
        "menu command exited with {}",output.status
    ).into());}
    let result=String::from_utf8(output.stdout)?;
    let selected=mako_menu_selected_line(&result).ok_or("no action selected")?;
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
    notification_json_text_with_actions(map,&action_pairs(map))
}

fn notification_json_text_with_actions(
    map:&HashMap<String,OwnedValue>,actions:&[(String,String)]
)->String{
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
    notification_text_with_actions(map,&action_pairs(map))
}
fn notification_text_with_actions(
    map:&HashMap<String,OwnedValue>,actions:&[(String,String)]
)->String{
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
    if !actions.is_empty(){
        result.push_str("  Actions:\n");
        for (key,label) in actions {
            result.push_str(&format!("    {key}: {label}\n"));
        }
    }
    result
}

pub async fn execute(command: Command) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if command == Command::Help {
        // Upstream makoctl prints its usage to stderr, not stdout.
        eprint!("{USAGE}");
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
            let records:Vec<OrderedControlRecord>=proxy.call(method,&()).await?;
            if json {
                let mut out=String::from("[");
                for (i,record) in records.iter().enumerate(){
                    if i>0{out.push(',');}
                    out.push('\n');
                    out.push_str(&notification_json_text_with_actions(
                        &record.fields,&record.actions));
                }
                out.push_str("\n]\n");
                print!("{out}");
            } else {
                for record in &records {
                    print!("{}",notification_text_with_actions(
                        &record.fields,&record.actions));
                }
            }
        }
        Command::Menu {id,program} => {
            let records:Vec<OrderedControlRecord>=
                proxy.call("ListNotifications",&()).await?;
            let note=records.iter().find(|record| {
                id==0 || record.fields.get("id")
                    .and_then(|v|u32::try_from(v).ok())==Some(id)
            }).ok_or("notification not found")?;
            let notification_id=note.fields.get("id")
                .and_then(|v|u32::try_from(v).ok()).ok_or("missing notification ID")?;
            if note.actions.is_empty(){return Err("notification has no actions".into());}
            let action=choose_from_menu(&program,&note.actions)?;
            let _: ()=proxy.call("InvokeAction",&(notification_id,action)).await?;
        }
        Command::Reload => {let _: ()=proxy.call("Reload",&()).await?;}
        Command::Modes {edit} => {
            let modes: Vec<String>=proxy.call("ListModes",&()).await?;
            if edit.is_empty() {
                print!("{}",modes_text(&modes));
            } else {
                let replacement=update_modes(modes,&edit);
                let _: ()=proxy.call("SetModes",&(replacement.clone(),)).await?;
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
    fn bare_makoctl_usage_is_failure_but_explicit_help_succeeds(){
        assert_eq!(parse_command(&[]).unwrap_err(),USAGE);
        assert!(USAGE.contains("dismiss [-n id]"));
        assert!(USAGE.contains("mode -s mode..."));
        for args in [vec!["help"],vec!["-h"],vec!["--help"]]{
            let words=args.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(parse_command(&words).unwrap(),Command::Help);
        }
    }

    #[test]
    fn lone_dash_is_positional_under_mako_gnu_getopt(){
        assert_eq!(parse_command(&v(&["invoke","-"])).unwrap(),
            Command::Invoke{id:0,action:"-".into()});
        assert_eq!(parse_command(&v(&["invoke","--","-n"])).unwrap(),
            Command::Invoke{id:0,action:"-n".into()});
        assert_eq!(parse_command(&v(&["mode","-s","-"])).unwrap(),
            Command::Modes{edit:vec![('s',"".into()),('S',"-".into())]});
        assert!(parse_command(&v(&["mode","-"])).is_err(),
            "bare positional mode still requires -s");
    }

    #[test]
    fn menu_getline_preserves_cr_and_reads_only_first_line(){
        assert_eq!(mako_menu_selected_line("Open\nIgnore\n"),Some("Open"));
        assert_eq!(mako_menu_selected_line("Open\r\nIgnore"),Some("Open\r"));
        assert_eq!(mako_menu_selected_line("Open"),Some("Open"));
        assert_eq!(mako_menu_selected_line(""),None);
        let actions=vec![("chosen".into(),"Open".into())];
        assert!(selected_action_key(&actions,
            mako_menu_selected_line("Open\r\n").unwrap()).is_none());
        assert_eq!(selected_action_key(&actions,
            mako_menu_selected_line("Open\n").unwrap()),Some("chosen".into()));
    }

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
    fn unknown_urgency_has_null_json_and_no_human_label(){
        let fields=HashMap::from([
            ("id".to_owned(),OwnedValue::from(12u32)),
            ("urgency".to_owned(),OwnedValue::from(255u8)),
        ]);
        let json=notification_json_text(&fields);
        assert!(json.contains("\"urgency\": null"),"{json}");
        assert!(!notification_text(&fields).contains("Urgency:"));
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
        assert_eq!(parse_command(&v(&["mode","-sss","quiet"])).unwrap(),
            Command::Modes{edit:vec![('s',"".into()),('S',"quiet".into())]});
        assert_eq!(parse_command(&v(&["mode","-ss"])).unwrap(),
            Command::Modes{edit:vec![('s',"".into())]});
        assert!(parse_command(&v(&["mode","quiet"])).is_err());
        assert!(parse_command(&v(&["mode","-a"])).is_err());
        assert_eq!(parse_command(&v(&["mode","-a","-t"])).unwrap(),
            Command::Modes{edit:vec![('a',"-t".into())]});
        assert_eq!(parse_command(&v(&["mode","-aquiet","-t-quiet"])).unwrap(),
            Command::Modes{edit:vec![('a',"quiet".into()),('t',"-quiet".into())]});
        assert!(parse_command(&v(&["mode","-aquiet","-s"])).is_err());
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
        // GNU getopt permutes -n options before the positional menu argv.
        assert_eq!(parse_command(&v(&["menu","fuzzel","-n","27","--","-d"])).unwrap(),
            Command::Menu{id:27,program:vec!["fuzzel".into(),"-d".into()]});
        assert_eq!(parse_command(&v(&["menu","fuzzel","--","-n","27"])).unwrap(),
            Command::Menu{id:0,program:vec!["fuzzel".into(),"-n".into(),"27".into()]});
        assert_eq!(parse_command(&v(&["menu","fuzzel","-n27"])).unwrap(),
            Command::Menu{id:27,program:vec!["fuzzel".into()]});
        assert!(parse_command(&v(&["menu","fuzzel","-x"])).is_err());
    }

    #[test]
    fn selecting_menu_title_recovers_correct_action_key() {
        let actions=vec![
            ("default".into(),"Open".into()),
            ("dismiss".into(),"Ignore".into())
        ];
        assert_eq!(selected_action_key(&actions,"Open"),Some("default".into()));
        assert!(selected_action_key(&actions,"no such title").is_none());
        let chosen=choose_from_menu(&["cat".into()],&actions).unwrap();
        assert_eq!(chosen,"default");
        let duplicates=vec![
            ("later".into(),"Shared title".into()),
            ("earlier".into(),"Shared title".into()),
        ];
        assert_eq!(selected_action_key(&duplicates,"Shared title"),
            Some("later".into()));
    }

    #[test]
    fn makoctl_text_and_json_keep_ordered_duplicate_action_keys(){
        let mut fields=HashMap::new();
        fields.insert("id".to_owned(),OwnedValue::from(17u32));
        let actions=vec![
            ("z".to_owned(),"Latest".to_owned()),
            ("a".to_owned(),"Middle".to_owned()),
            ("z".to_owned(),"Earliest".to_owned()),
        ];
        let human=notification_text_with_actions(&fields,&actions);
        assert!(human.contains(
            "  Actions:\n    z: Latest\n    a: Middle\n    z: Earliest\n"
        ),"{human}");
        let json=notification_json_text_with_actions(&fields,&actions);
        assert!(json.contains(
            r#""actions": {"z": "Latest", "a": "Middle", "z": "Earliest"}"#
        ),"{json}");
    }

    #[test]
    fn menu_list_and_json_follow_stable_zvariant_dictionary_order(){
        use zbus::zvariant::{Dict,Signature};
        let mut dict=Dict::new(&Signature::Str,&Signature::Str);
        for (key,label) in [
            ("z","Last"),("a","First"),("z","Duplicate"),
        ]{
            dict.append(Value::from(key),Value::from(label)).unwrap();
        }
        let mut fields=HashMap::new();
        fields.insert("id".into(),OwnedValue::from(8u32));
        fields.insert("actions".into(),Value::Dict(dict).try_to_owned().unwrap());
        assert_eq!(action_pairs(&fields),vec![
            ("a".into(),"First".into()),
            ("z".into(),"Duplicate".into()),
        ]);
        assert!(notification_text(&fields).contains(
            "  Actions:\n    a: First\n    z: Duplicate\n"
        ));
        assert!(notification_json_text(&fields).contains(
            r#""actions": {"a": "First", "z": "Duplicate"}"#
        ));
    }

    #[test]
    fn dismiss_getopt_long_accepts_clusters_permutation_and_terminator() {
        assert_eq!(parse_command(&v(&["dismiss","-hn42"])).unwrap(),
            Command::Dismiss{id:42,all:false,group:false,history:false});
        assert_eq!(parse_command(&v(&["dismiss","--all","-h"])).unwrap(),
            Command::Dismiss{id:0,all:true,group:false,history:false});
        assert_eq!(parse_command(&v(&["dismiss","--al","--no-h"])).unwrap(),
            Command::Dismiss{id:0,all:true,group:false,history:false});
        assert_eq!(parse_command(&v(&["dismiss","--gr","--no"])).unwrap(),
            Command::Dismiss{id:0,all:false,group:true,history:false});
        assert!(parse_command(&v(&["dismiss","--a","--g"])).is_err());
        assert!(parse_command(&v(&["dismiss","--history"])).is_err());
        assert_eq!(parse_command(&v(&["dismiss","-a","ignored"])).unwrap(),
            Command::Dismiss{id:0,all:true,group:false,history:true});
        assert_eq!(parse_command(&v(&["dismiss","--","-a"])).unwrap(),
            Command::Dismiss{id:0,all:false,group:false,history:true});
        assert!(parse_command(&v(&["dismiss","-ag"])).is_err());
        assert!(parse_command(&v(&["dismiss","-n42","-g"])).is_err());
        assert!(parse_command(&v(&["dismiss","-z"])).is_err());
    }

    #[test]
    fn notification_ids_match_mako_strtoumax_cli_semantics(){
        assert_eq!(parse_mako_id("  +0042"),Ok(42));
        assert_eq!(parse_mako_id("\x0b+0042"),Ok(42));
        assert_eq!(parse_mako_id("\x0c0"),Ok(0));
        assert_eq!(parse_mako_id("\t-0"),Ok(0));
        assert_eq!(parse_mako_id(" -0000"),Ok(0));
        assert!(parse_mako_id("-0001").is_err());
        assert_eq!(parse_mako_id(""),Ok(0));
        assert!(parse_mako_id(" ").is_err());
        assert!(parse_mako_id("42 ").is_err());
        assert!(parse_mako_id("-1").is_err());
        assert!(parse_mako_id("4294967296").is_err());
        assert_eq!(parse_command(&v(&["invoke","-n"," 42"])).unwrap(),
            Command::Invoke{id:42,action:"default".into()});
        assert_eq!(parse_command(&v(&["menu","-n"," +42","cat"])).unwrap(),
            Command::Menu{id:42,program:vec!["cat".into()]});
        assert_eq!(parse_command(&v(&["dismiss","-n","\t-0"])).unwrap(),
            Command::Dismiss{id:0,all:false,group:false,history:true});
    }

    #[test]
    fn accepts_mako_list_and_default_invoke() {
        assert_eq!(parse_command(&v(&["history","-j"])).unwrap(),
            Command::List{history:true,json:true});
        assert_eq!(parse_command(&v(&["history","-jj","-j"])).unwrap(),
            Command::List{history:true,json:true});
        assert_eq!(parse_command(&v(&["list","--","-j"])).unwrap(),
            Command::List{history:false,json:false});
        assert_eq!(parse_command(&v(&["list","extra","-jj"])).unwrap(),
            Command::List{history:false,json:true});
        assert_eq!(parse_command(&v(&["history","extra"])).unwrap(),
            Command::List{history:true,json:false});
        assert_eq!(parse_command(&v(&["history","extra","--","-j"])).unwrap(),
            Command::List{history:true,json:false});
        assert_eq!(parse_command(&v(&["restore","ignored","--bad"])).unwrap(),
            Command::Restore);
        assert_eq!(parse_command(&v(&["reload","ignored"])).unwrap(),
            Command::Reload);
        assert!(parse_command(&v(&["list","-q"])).is_err());
        assert_eq!(parse_command(&v(&["invoke"])).unwrap(),
            Command::Invoke{id:0,action:"default".into()});
        assert_eq!(parse_command(&v(&["invoke","open","-n34"])).unwrap(),
            Command::Invoke{id:34,action:"open".into()});
        assert_eq!(parse_command(&v(&["invoke","--","-open","ignored"])).unwrap(),
            Command::Invoke{id:0,action:"-open".into()});
        assert_eq!(parse_command(&v(&["invoke","default","next","-n3"])).unwrap(),
            Command::Invoke{id:3,action:"default".into()});
        assert!(parse_command(&v(&["invoke","--bad"])).is_err());
        assert_eq!(parse_command(&v(&["invoke","-n34","default"])).unwrap(),
            Command::Invoke{id:34,action:"default".into()});
    }
}
