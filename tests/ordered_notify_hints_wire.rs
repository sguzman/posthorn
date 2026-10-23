//! Isolated parser experiment for Mako's arrival-ordered Notify a{sv} hints.
//! D-Bus dictionaries are conventionally unique-keyed, but Mako's C loop
//! consumes entries in wire order and later aliases override earlier ones.

use serde::de::{Deserialize,Deserializer,MapAccess,Visitor};
use serde::ser::{Serialize,SerializeMap,Serializer};
use std::fmt;
use zbus::zvariant::{serialized::Context,OwnedValue,Type,Value,LE};

#[derive(Debug,Type)]
#[zvariant(signature="a{sv}")]
struct OrderedHints(Vec<(String,OwnedValue)>);

impl Serialize for OrderedHints{
    fn serialize<S:Serializer>(&self,serializer:S)->Result<S::Ok,S::Error>{
        let mut map=serializer.serialize_map(Some(self.0.len()))?;
        for (key,value) in &self.0{
            map.serialize_entry(key,value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for OrderedHints{
    fn deserialize<D:Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
        struct Entries;
        impl<'de> Visitor<'de> for Entries{
            type Value=OrderedHints;
            fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result{
                f.write_str("arrival-ordered D-Bus a{sv} hint dictionary")
            }
            fn visit_map<A:MapAccess<'de>>(self,mut map:A)->Result<Self::Value,A::Error>{
                let mut entries=Vec::new();
                while let Some((key,value))=map.next_entry::<String,OwnedValue>()?{
                    entries.push((key,value));
                }
                Ok(OrderedHints(entries))
            }
        }
        deserializer.deserialize_map(Entries)
    }
}

#[test]
fn duplicate_and_alias_hint_entries_keep_their_original_wire_order(){
    assert_eq!(OrderedHints::SIGNATURE.to_string(),"a{sv}");
    let to_owned=|text:&str|OwnedValue::try_from(Value::from(text)).unwrap();
    let hints=OrderedHints(vec![
        ("image_path".into(),to_owned("first.png")),
        ("image-path".into(),to_owned("second.png")),
        ("image_path".into(),to_owned("")),
    ]);
    let data=zbus::zvariant::to_bytes(Context::new_dbus(LE,0),&hints).unwrap();
    let (parsed,_):(OrderedHints,_)=data.deserialize().unwrap();
    assert_eq!(parsed.0.len(),3,"ordered deserializer must retain duplicate keys");
    let values:Vec<_>=parsed.0.iter().map(|(_,v)|
        String::try_from(v.try_clone().unwrap()).unwrap()).collect();
    assert_eq!(values,["first.png","second.png",""]);
    assert_eq!(parsed.0.iter().map(|(k,_)|k.as_str()).collect::<Vec<_>>(),
        ["image_path","image-path","image_path"]);
}
