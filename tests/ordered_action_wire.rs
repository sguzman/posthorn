//! A small D-Bus encoding experiment against Mako's a{ss} action field.
//! The production Mako control API must retain aa{sv} and a{ss} signatures;
//! no user notification content or private workstation paths are involved.

use serde::ser::{Serialize, SerializeMap, Serializer};
use zbus::zvariant::{serialized::Context, SerializeValue, Type, LE};
use std::collections::HashMap;

#[derive(Debug, Type)]
#[zvariant(signature = "a{ss}")]
struct OrderedActionDict(Vec<(String,String)>);

impl Serialize for OrderedActionDict {
    fn serialize<S:Serializer>(&self, serializer:S)->Result<S::Ok,S::Error>{
        let mut map=serializer.serialize_map(Some(self.0.len()))?;
        for (key,label) in &self.0{
            map.serialize_entry(key,label)?;
        }
        map.end()
    }
}

#[derive(Debug, Type)]
#[zvariant(signature = "a{sv}")]
struct ActionEnvelope{ actions:OrderedActionDict }

impl Serialize for ActionEnvelope {
    fn serialize<S:Serializer>(&self,serializer:S)->Result<S::Ok,S::Error>{
        let mut map=serializer.serialize_map(Some(1))?;
        map.serialize_entry("actions",&SerializeValue(&self.actions))?;
        map.end()
    }
}

fn offset(haystack:&[u8],needle:&[u8])->usize{
    haystack.windows(needle.len()).position(|part|part==needle)
        .expect("expected D-Bus string bytes missing")
}

#[test]
fn custom_serde_map_preserves_action_order_and_duplicate_entries(){
    let pairs=OrderedActionDict(vec![
        ("zebra".into(),"First".into()),
        ("alpha".into(),"Middle".into()),
        ("zebra".into(),"Last".into()),
    ]);
    let ctxt=Context::new_dbus(LE,0);
    let encoded=zbus::zvariant::to_bytes(ctxt,&pairs).unwrap();
    let bytes=encoded.bytes();
    assert!(offset(bytes,b"zebra\0")<offset(bytes,b"alpha\0"));
    assert!(offset(bytes,b"First\0")<offset(bytes,b"Middle\0"));
    assert!(offset(bytes,b"Middle\0")<offset(bytes,b"Last\0"));
    // A HashMap consumer discards duplicate keys by its own choice; the
    // underlying wire has BOTH entries, like Mako's sd_bus producer.
    let (decoded,_): (HashMap<String,String>,_) = encoded.deserialize().unwrap();
    assert_eq!(decoded.len(),2);
}

#[test]
fn nested_variant_keeps_original_action_order_under_a_sv(){
    let record=ActionEnvelope{actions:OrderedActionDict(vec![
        ("zebra".into(),"First".into()),
        ("alpha".into(),"Middle".into()),
        ("zebra".into(),"Last".into()),
    ])};
    let ctxt=Context::new_dbus(LE,0);
    let encoded=zbus::zvariant::to_bytes(ctxt,&record).unwrap();
    let bytes=encoded.bytes();
    assert!(offset(bytes,b"First\0")<offset(bytes,b"Middle\0"));
    assert!(offset(bytes,b"Middle\0")<offset(bytes,b"Last\0"));
    let (decoded,_):(HashMap<String,zbus::zvariant::OwnedValue>,_)=
        encoded.deserialize().unwrap();
    assert!(decoded.contains_key("actions"));
}
