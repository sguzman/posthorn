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
        let actions = actions
            .chunks_exact(2)
            .map(|pair| Action {
                key: pair[0].clone(),
                label: pair[1].clone(),
            })
            .collect();

        // A hint that isn't a string is simply not the requested hint.
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
        let icon_path = ["image-path","image_path"].into_iter()
            .map(text_hint).find(|s|!s.is_empty())
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
            progress: hints.get("value")
                .and_then(|v|i32::try_from(v).ok())
                .unwrap_or(0).clamp(0,100) as u8,
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
        // Mako advertises capabilities from its root style, not from
        // currently visible category rules and not as a fixed list.
        let config=self.runtime.as_ref().map(SharedRuntime::config);
        let style=config.as_ref().map(|c|&c.global);
        let option=|key:&str|style.and_then(|s|s.get(key)).map(String::as_str);
        let enabled=|key:&str,default:bool|option(key)
            .map(|v|v=="1"||v.eq_ignore_ascii_case("true")).unwrap_or(default);
        let mut capabilities=Vec::new();
        if option("format").unwrap_or("<b>%s</b>\\n%b").contains("%b"){
            capabilities.push("body");
        }
        if enabled("markup",true){capabilities.push("body-markup");}
        if enabled("actions",true){capabilities.push("actions");}
        if enabled("icons",true){capabilities.push("icon-static");}
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
    fn progress_hint_is_clamped_and_replacement_resets_when_absent() {
        let svc=service();
        let hints=[("value".into(),OwnedValue::from(170i32))].into();
        let id=svc.apply_notify("app",0,"","Working","",Vec::new(),hints,-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].progress,100);
        svc.apply_notify("app",id,"","Finished","",Vec::new(),HashMap::new(),-1).unwrap();
        assert_eq!(svc.store.lock().unwrap().active()[0].progress,0);
    }

    #[test]
    fn capabilities_track_mako_root_style_not_category_rules() {
        let service=FreedesktopNotifications::with_runtime(
            SharedRuntime::new(crate::config::Config::parse(
                "format=%s\nmarkup=0\nactions=0\nicons=0\n                 [category=success]\nmarkup=1\nactions=1\nicons=1\n"
            ).unwrap())
        );
        assert_eq!(service.get_capabilities(),
            ["x-canonical-private-synchronous","x-dunst-stack-tag"]);
        let default=FreedesktopNotifications::with_runtime(
            SharedRuntime::new(crate::config::Config::default())
        );
        assert_eq!(default.get_capabilities(),
            ["body","body-markup","actions","icon-static",
             "x-canonical-private-synchronous","x-dunst-stack-tag"]);
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
