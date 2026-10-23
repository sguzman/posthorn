//! Freedesktop notification D-Bus adapter on an isolated development bus.
//!
//! Preserves Mako wire signatures, including action and activation signals.
//! The real session-bus name remains reserved for future MVP inauguration;
//! do not run a replacement alongside the installed Mako daemon.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::{interface, object_server::SignalEmitter, zvariant::OwnedValue};

use crate::notification::{Action, CloseReason, NotificationStore, Request, Urgency};
use crate::runtime::SharedRuntime;
use std::time::Instant;

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

/// State shared by the desktop D-Bus API and (eventually) the Wayland runtime.
#[derive(Clone, Debug)]
pub struct FreedesktopNotifications {
    store: Arc<Mutex<NotificationStore>>,
    runtime: Option<SharedRuntime>,
}

impl FreedesktopNotifications {
    pub fn new(store: Arc<Mutex<NotificationStore>>) -> Self {
        Self { store, runtime: None }
    }

    pub fn with_runtime(runtime: SharedRuntime) -> Self {
        Self { store: Arc::clone(&runtime.store), runtime: Some(runtime) }
    }

    pub fn store(&self) -> &Arc<Mutex<NotificationStore>> {
        &self.store
    }

    fn apply_notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        // Freedesktop actions are alternating key/label pairs.
        if actions.len() % 2 != 0 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "notification actions must be key/label pairs".into(),
            ));
        }
        // Upstream appends each action using wl_list_insert at the head:
        // the internal action list is reversed relative to sender order.
        let actions = actions
            .chunks_exact(2).rev()
            .map(|pair| Action {
                key: pair[0].clone(),
                label: pair[1].clone(),
            })
            .collect();

        // Upstream Mako reads known hint variants with their exact wire type.
        // Reject mismatches *before* mutating a live or replaced notification.
        // Unknown extension hints are ignored, as in the C implementation.
        for (name,value) in &hints {
            let valid=match name.as_str(){
                "category"|"desktop-entry"|"image-path"|"image_path"
                    |"x-dunst-stack-tag"|"x-canonical-private-synchronous"=>
                    String::try_from(value.clone()).is_ok(),
                "value"=>i32::try_from(value).is_ok(),
                "urgency"=>u8::try_from(value).is_ok()
                    ||u32::try_from(value).is_ok()
                    ||i32::try_from(value).is_ok(),
                "image-data"|"image_data"|"icon_data"=>{
                    let tuple:Option<(i32,i32,i32,bool,i32,i32,Vec<u8>)>=
                        value.try_clone().ok()
                            .and_then(|v|v.try_into().ok());
                    tuple.is_some()
                }
                _=>true,
            };
            if !valid{
                return Err(zbus::fdo::Error::InvalidArgs(
                    format!("notification hint '{name}' has incorrect D-Bus type")
                ));
            }
        }

        // Known string hint types have been checked above.
        let text_hint = |name: &str| {
            hints
                .get(name)
                .and_then(|hint| String::try_from(hint.clone()).ok())
                .unwrap_or_default()
        };

        let mut tag = text_hint("x-dunst-stack-tag");
        if tag.is_empty() {
            tag = text_hint("x-canonical-private-synchronous");
        }
        // Mako accepts byte ('y'), uint32 ('u', notably Chromium), and
        // signed int32 ('i') urgency variants. Checking only 'y' silently
        // treats real Chromium critical/low notifications as normal.
        let urgency_number = hints.get("urgency").and_then(|v| {
            u8::try_from(v).ok().map(i64::from)
                .or_else(||u32::try_from(v).ok().map(i64::from))
                .or_else(||i32::try_from(v).ok().map(i64::from))
        });
        let urgency = match urgency_number {
            None | Some(-1) => Urgency::Unknown,
            Some(0) => Urgency::Low,
            Some(2) => Urgency::Critical,
            _ => Urgency::Normal,
        };
        // Historical Freedesktop aliases are still accepted by Mako. The
        // modern image-data hint takes precedence over both image-path and
        // the app_icon Notify argument.
        let raw_hint = ["image-data","image_data","icon_data"].into_iter()
            .find_map(|name| hints.get(name));
        let image_data = raw_hint.and_then(|hint| {
            let data: (i32,i32,i32,bool,i32,i32,Vec<u8>) =
                hint.try_clone().ok()?.try_into().ok()?;
            crate::icons::decode_raw_image(
                data.0,data.1,data.2,data.3,data.4,data.5,&data.6)
        });
        // Mako unconditionally overwrites app_icon when image-path is
        // present, even if its value is empty. An empty hint deliberately
        // removes an icon; it must not resurrect the Notify app_icon.
        let icon_path = ["image-path","image_path"].into_iter()
            .find(|name|hints.contains_key(*name))
            .map(text_hint)
            .unwrap_or_else(||app_icon.to_string());
        let request = Request {
            replaces_id,
            app_name: app_name.to_string(),
            app_icon: icon_path,
            image_data,
            summary: summary.to_string(),
            body: body.to_string(),
            category: text_hint("category"),
            desktop_entry: text_hint("desktop-entry"),
            tag,
            urgency,
            // Mako stores the signed D-Bus value directly and defaults to
            // -1 when omitted; only Cairo clamps the visible fill width.
            progress: hints.get("value")
                .and_then(|v|i32::try_from(v).ok()).unwrap_or(-1),
            requested_timeout_ms: expire_timeout,
            actions,
        };
        let mut state = self
            .store
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("notification state lock poisoned".into()))?;
        let id = state.push(request);
        drop(state);
        if let Some(runtime) = &self.runtime {
            runtime.schedule(id, Instant::now())
                .map_err(|e| zbus::fdo::Error::Failed(e.into()))?;
        }
        Ok(id)
    }
}

#[interface(name = "org.freedesktop.Notifications")]
impl FreedesktopNotifications {
    #[zbus(name = "GetCapabilities")]
    fn get_capabilities(&self) -> Vec<String> {
        // Upstream Mako creates config.superstyle by collecting the union
        // of root, built-in and user criteria styles. Its advertised features
        // describe what ANY notification may render, not just the root.
        let config=self.runtime.as_ref().map(SharedRuntime::config)
            .unwrap_or_else(||std::sync::Arc::new(crate::config::Config::default()));
        let styles=std::iter::once(&config.global)
            .chain(config.rules.iter().map(|rule|&rule.options));
        // Mako always contributes its built-in [grouped] format
        // "(%g) <b>%s</b>\\n%b" to superstyle, so body is advertised
        // even if the root and all user criteria replace %b.
        let mut body=true;
        let (mut markup,mut actions,mut icons)=(false,false,false);
        for style in styles {
            if style.get("format").is_some_and(|f|f.contains("%b")){body=true;}
            for (key,target) in [
                ("markup",&mut markup),("actions",&mut actions),("icons",&mut icons)
            ]{
                let enabled=style.get(key)
                    .is_some_and(|v|v=="1"||v.eq_ignore_ascii_case("true"));
                // Root style supplies the defaults; omitted scoped options
                // must not turn an already-true capability off.
                *target |= enabled;
            }
        }
        // Mako's built-in root icons default to true, including when this
        // parser has no explicit "icons" entry in its defaults map.
        if !config.global.contains_key("icons"){icons=true;}
        let mut capabilities=Vec::new();
        if body{capabilities.push("body");}
        if markup{capabilities.push("body-markup");}
        if actions{capabilities.push("actions");}
        if icons{capabilities.push("icon-static");}
        capabilities.extend(["x-canonical-private-synchronous","x-dunst-stack-tag"]);
        capabilities.into_iter().map(str::to_string).collect()
    }

    #[zbus(name = "GetServerInformation")]
    fn get_server_information(&self) -> (String, String, String, String) {
        // Exact Mako 1.11.0 wire-compatible response; identity changes after
        // MVP require an explicit compatibility decision.
        ("mako".into(), "emersion".into(), "0.0.0".into(), "1.2".into())
    }

    #[zbus(name = "Notify")]
    async fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<u32> {
        let id = self.apply_notify(
            app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout,
        )?;
        if let Some(runtime)=&self.runtime {
            let binding=runtime.on_notify_binding(id)
                .map_err(|e|zbus::fdo::Error::Failed(e.into()))?;
            // Mako invokes every on-notify binding after assigning groups.
            // Shell hooks are still limited to trusted configuration strings;
            // no sender-provided content enters the shell.
            match binding {
                crate::hooks::NotifyBinding::None=>{},
                crate::hooks::NotifyBinding::Exec(command)=>{
                    if let Err(err)=crate::hooks::NotifyBinding::Exec(command)
                        .run_exec(id) {
                        eprintln!("posthorn on-notify hook failed: {err}");
                    }
                }
                other=>crate::input::dispatch_binding(
                    conn,runtime,id,other,None
                ).await.map_err(|e|zbus::fdo::Error::Failed(e.to_string()))?,
            }
        }
        crate::signals::invalidate(conn, "Notifications").await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(id)
    }

    #[zbus(name = "CloseNotification")]
    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let closed = if let Some(runtime) = &self.runtime {
            runtime.close(id, CloseReason::ClosedByCall, true)
                .map_err(|e| zbus::fdo::Error::Failed(e.into()))?
        } else {
            let mut state = self.store.lock().map_err(|_| {
                zbus::fdo::Error::Failed("notification state lock poisoned".into())
            })?;
            state.close(id, CloseReason::ClosedByCall, true, true).is_some()
        };
        if closed {
            Self::notification_closed(&emitter, id, CloseReason::ClosedByCall as u32)
                .await
                .map_err(|err| zbus::fdo::Error::Failed(err.to_string()))?;
            crate::signals::invalidate(emitter.connection(), "Notifications")
                .await.map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> FreedesktopNotifications {
        FreedesktopNotifications::new(Arc::new(Mutex::new(NotificationStore::new(5))))
    }

    #[test]
    fn sender_action_pairs_are_stored_in_upstream_reverse_order() {
        let svc=service();
        let actions=vec![
            "default".into(),"First".into(),
            "other".into(),"Middle".into(),
            "default".into(),"Last".into(),
        ];
        let id=svc.apply_notify("sender",0,"","Action order","",
            actions,HashMap::new(),-1).unwrap();
        let store=svc.store.lock().unwrap();
        let note=store.active().iter().find(|n|n.id==id).unwrap();
        assert_eq!(note.actions.iter().map(|a|a.label.as_str())
            .collect::<Vec<_>>(),vec!["Last","Middle","First"]);
    }

    #[test]
    fn progress_hint_preserves_signed_value_and_replacement_resets_unknown() {
        let svc=service();
        let hints=[("value".into(),OwnedValue::from(170i32))].into();
        let id=svc.apply_notify("app",0,"","Working","",Vec::new(),hints,-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].progress,170);
        let hints=[("value".into(),OwnedValue::from(-9i32))].into();
        svc.apply_notify("app",id,"","Replaced","",Vec::new(),hints,-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].progress,-9);
        svc.apply_notify("app",id,"","Finished","",Vec::new(),HashMap::new(),-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].progress,-1);
    }

    #[test]
    fn capabilities_are_superset_of_all_mako_criteria_styles() {
        let service=FreedesktopNotifications::with_runtime(
            SharedRuntime::new(crate::config::Config::parse(
                "format=%s\nmarkup=0\nactions=0\nicons=0\n                 [category=success]\nmarkup=1\nactions=1\nicons=1\n"
            ).unwrap())
        );
        assert_eq!(service.get_capabilities(),
            ["body","body-markup","actions","icon-static",
             "x-canonical-private-synchronous","x-dunst-stack-tag"]);
        let scoped_body=FreedesktopNotifications::with_runtime(
            SharedRuntime::new(crate::config::Config::parse(
                "format=%s\nmarkup=0\nactions=0\nicons=0\n[category=success]\nformat=%b\n"
            ).unwrap())
        );
        assert_eq!(scoped_body.get_capabilities(),
            ["body","x-canonical-private-synchronous","x-dunst-stack-tag"]);
        let default=FreedesktopNotifications::with_runtime(
            SharedRuntime::new(crate::config::Config::default())
        );
        assert_eq!(default.get_capabilities(),
            ["body","body-markup","actions","icon-static",
             "x-canonical-private-synchronous","x-dunst-stack-tag"]);
    }

    #[test]
    fn absence_of_urgency_preserves_upstream_unknown_state() {
        let svc=service();
        let id=svc.apply_notify("sender",0,"","No urgency","",Vec::new(),
            HashMap::new(),-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active().iter()
            .find(|n|n.id==id).unwrap().urgency,Urgency::Unknown);
        let other=svc.apply_notify("sender",0,"","Explicit normal","",Vec::new(),
            [("urgency".into(),OwnedValue::from(1u8))].into(),-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active().iter()
            .find(|n|n.id==other).unwrap().urgency,Urgency::Normal);
    }

    #[test]
    fn chromium_uint32_and_signed_int32_urgency_hints_match_mako() {
        let service=service();
        for (variant,expected) in [
            (OwnedValue::from(0u8),Urgency::Low),
            (OwnedValue::from(2u8),Urgency::Critical),
            (OwnedValue::from(0u32),Urgency::Low),
            (OwnedValue::from(2u32),Urgency::Critical),
            (OwnedValue::from(0i32),Urgency::Low),
            (OwnedValue::from(2i32),Urgency::Critical),
            (OwnedValue::from(1i32),Urgency::Normal),
            (OwnedValue::from(-1i32),Urgency::Unknown),
        ] {
            let hints=[("urgency".into(),variant)].into();
            let id=service.apply_notify("sender",0,"","Urgency","",
                Vec::new(),hints,0).unwrap();
            let active=service.store.lock().unwrap();
            assert_eq!(active.active().iter().find(|n|n.id==id).unwrap().urgency,expected);
        }
    }

    #[test]
    fn maps_category_urgency_actions_and_tags() {
        let s = service();
        let hints: HashMap<_, _> = [
            ("category".into(), OwnedValue::try_from(zbus::zvariant::Value::from("success")).unwrap()),
            ("urgency".into(), OwnedValue::from(2u8)),
            ("x-dunst-stack-tag".into(), OwnedValue::try_from(zbus::zvariant::Value::from("job")).unwrap()),
        ].into();
        let id = s.apply_notify("worker", 0, "", "Done", "Completed",
            vec!["default".into(), "Open".into()], hints, 7000).unwrap();
        let store = s.store.lock().unwrap();
        let n = store.active().iter().find(|n| n.id == id).unwrap();
        assert_eq!(n.category, "success");
        assert_eq!(n.tag, "job");
        assert_eq!(n.urgency, Urgency::Critical);
        assert_eq!(n.actions[0].key, "default");
        assert_eq!(n.requested_timeout_ms, 7000);
    }

    #[test]
    fn hint_image_path_overrides_notify_app_icon_and_preserves_replacement() {
        let service=service();
        let hints=[(
            "image-path".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from("/tmp/posthorn-fixture.png")).unwrap(),
        )].into();
        let id=service.apply_notify(
            "app",0,"ignored-icon-name","First","",
            Vec::new(),hints,-1
        ).unwrap();
        assert_eq!(service.store.lock().unwrap().active()[0].app_icon,
            "/tmp/posthorn-fixture.png");
        let second=service.apply_notify(
            "app",id,"another-icon","Replacement","",
            Vec::new(),HashMap::new(),-1
        ).unwrap();
        assert_eq!(second,id);
        assert_eq!(service.store.lock().unwrap().active()[0].app_icon,
            "another-icon");
    }

    #[test]
    fn empty_image_path_hint_clears_sender_icon_including_replacements(){
        let svc=service();
        let empty=||[(
            "image-path".into(),
            OwnedValue::try_from(zbus::zvariant::Value::from("")).unwrap(),
        )].into();
        let id=svc.apply_notify(
            "sender",0,"existing-icon","First","",Vec::new(),empty(),-1
        ).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].app_icon,"");
        svc.apply_notify(
            "sender",id,"replacement-icon","Second","",Vec::new(),empty(),-1
        ).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].app_icon,"");
        svc.apply_notify(
            "sender",id,"again","Third","",Vec::new(),[
                ("image_path".into(),
                 OwnedValue::try_from(zbus::zvariant::Value::from("legacy-icon")).unwrap())
            ].into(),-1
        ).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].app_icon,"legacy-icon");
        svc.apply_notify("sender",id,"argument-icon","Fourth","",
            Vec::new(),HashMap::new(),-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].app_icon,"argument-icon");
    }

    #[test]
    fn image_data_hint_is_stored_with_correct_color_and_replaces_atomically() {
        let service=service();
        let tuple=(1i32,1i32,4i32,true,8i32,4i32,
            vec![255u8,128u8,0u8,128u8]);
        let value=OwnedValue::try_from(zbus::zvariant::Value::from(tuple)).unwrap();
        let hints=[("image-data".to_string(),value)].into();
        let id=service.apply_notify(
            "sender",0,"ignored","Inline Image","",
            Vec::new(),hints,-1
        ).unwrap();
        let image=service.store.lock().unwrap().active()[0]
            .image_data.clone().expect("image data decoded");
        assert_eq!(image.bgra.as_slice(), &[0,64,128,128]);
        service.apply_notify("sender",id,"new-icon","Changed","",
            Vec::new(),HashMap::new(),-1).unwrap();
        assert!(service.store.lock().unwrap().active()[0].image_data.is_none());
    }

    #[test]
    fn rejects_wrong_known_hint_types_without_mutating_notifications() {
        let service=service();
        let id=service.apply_notify(
            "sender",0,"","Original","",Vec::new(),
            HashMap::new(),-1
        ).unwrap();
        for (key,bad) in [
            ("category",OwnedValue::from(42i32)),
            ("desktop-entry",OwnedValue::from(true)),
            ("urgency",OwnedValue::try_from(zbus::zvariant::Value::from("2")).unwrap()),
            ("value",OwnedValue::from(90u32)),
            ("image-path",OwnedValue::from(99u8)),
            ("x-dunst-stack-tag",OwnedValue::from(false)),
        ] {
            let bad_hints=[(key.into(),bad)].into();
            assert!(matches!(service.apply_notify(
                "sender",id,"","Replaced","",Vec::new(),bad_hints,-1
            ),Err(zbus::fdo::Error::InvalidArgs(_))),"accepted invalid {key} hint");
            let store=service.store.lock().unwrap();
            assert_eq!(store.active().len(),1);
            assert_eq!(store.active()[0].id,id);
            assert_eq!(store.active()[0].summary,"Original");
        }
        // Sender-defined hints remain extensible and may have any type.
        let extension=[("third-party-custom-key".into(),OwnedValue::from(3i32))].into();
        assert_eq!(service.apply_notify(
            "sender",id,"","Updated","",Vec::new(),extension,-1
        ).unwrap(),id);
    }

    #[test]
    fn rejects_odd_action_arrays_without_touching_state() {
        let s = service();
        let result = s.apply_notify("a", 0, "", "oops", "", vec!["orphan".into()],
            HashMap::new(), -1);
        assert!(result.is_err());
        assert!(s.store.lock().unwrap().active().is_empty());
    }

    #[test]
    fn notification_replacement_keeps_identifier() {
        let s = service();
        let id = s.apply_notify("app", 0, "", "old", "", Vec::new(), HashMap::new(), -1).unwrap();
        let new_id = s.apply_notify("app", id, "", "new", "", Vec::new(), HashMap::new(), -1).unwrap();
        assert_eq!(id, new_id);
        assert_eq!(s.store.lock().unwrap().active()[0].summary, "new");
    }
}
