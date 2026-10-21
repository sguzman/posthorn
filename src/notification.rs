//! In-memory notification lifecycle, ported from Mako's `notification.c`.
//!
//! No SQLite, channels, or other post-MVP behavior belongs in this module.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Urgency { Low, Normal, Critical }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    ClosedByCall = 3,
    Undefined = 4,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Action {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub category: String,
    pub desktop_entry: String,
    pub tag: String,
    pub urgency: Urgency,
    pub requested_timeout_ms: i32,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub replaces_id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    pub category: String,
    pub desktop_entry: String,
    pub tag: String,
    pub urgency: Urgency,
    pub requested_timeout_ms: i32,
    pub actions: Vec<Action>,
}

impl Default for Request {
    fn default() -> Self {
        Self { replaces_id: 0, app_name: String::new(), app_icon: String::new(),
            summary: String::new(), body: String::new(), category: String::new(),
            desktop_entry: String::new(), tag: String::new(), urgency: Urgency::Normal,
            requested_timeout_ms: -1, actions: Vec::new() }
    }
}

impl Notification {
    fn from_request(id: u32, request: Request) -> Self {
        Self { id, app_name: request.app_name, app_icon: request.app_icon,
            summary: request.summary, body: request.body, category: request.category,
            desktop_entry: request.desktop_entry, tag: request.tag, urgency: request.urgency,
            requested_timeout_ms: request.requested_timeout_ms, actions: request.actions }
    }

    /// Returns None for a persistent notification, otherwise its timeout.
    pub fn timeout_ms(&self, default_ms: u32, ignore_sender: bool) -> Option<u32> {
        let timeout = if ignore_sender || self.requested_timeout_ms < 0 {
            default_ms
        } else { self.requested_timeout_ms as u32 };
        if timeout == 0 { None } else { Some(timeout) }
    }
}

/// Mirrors Mako's bounded, newest-first in-memory history and active list.
#[derive(Debug)]
pub struct NotificationStore {
    active: VecDeque<Notification>,
    history: VecDeque<Notification>,
    next_id: u32,
    max_history: usize,
}

impl NotificationStore {
    pub fn new(max_history: usize) -> Self {
        Self { active: VecDeque::new(), history: VecDeque::new(),
            next_id: 1, max_history }
    }

    pub fn active(&self) -> &VecDeque<Notification> { &self.active }
    pub fn history(&self) -> &VecDeque<Notification> { &self.history }

    pub fn push(&mut self, request: Request) -> u32 {
        // `replaces_id` only refers to currently active notifications.
        let existing = if request.replaces_id != 0 {
            self.active.iter().position(|n| n.id == request.replaces_id)
        } else { None };
        let existing = existing.or_else(|| {
            if request.tag.is_empty() { return None; }
            self.active.iter().position(|n|
                n.app_name == request.app_name && n.tag == request.tag)
        });
        if let Some(index) = existing {
            let id = self.active[index].id;
            self.active[index] = Notification::from_request(id, request);
            return id;
        }
        let id = self.allocate_id();
        self.active.push_front(Notification::from_request(id, request));
        id
    }

    fn allocate_id(&mut self) -> u32 {
        // D-Bus notification IDs must never be zero; avoid collisions at wraparound.
        loop {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            if self.next_id == 0 { self.next_id = 1; }
            if id != 0 && !self.active.iter().any(|n| n.id == id)
                && !self.history.iter().any(|n| n.id == id) { return id; }
        }
    }

    /// Returns closed notification for future D-Bus NotificationClosed emission.
    pub fn close(&mut self, id: u32, _reason: CloseReason, save_history: bool,
                 history_enabled: bool) -> Option<Notification> {
        let idx = self.active.iter().position(|n| n.id == id)?;
        let notification = self.active.remove(idx)?;
        if save_history && history_enabled && self.max_history != 0 {
            self.history.push_front(notification.clone());
            self.history.truncate(self.max_history);
        }
        Some(notification)
    }

    /// Restores the newest notification from the in-memory history.
    pub fn restore(&mut self) -> Option<u32> {
        let notification = self.history.pop_front()?;
        let id = notification.id;
        self.active.push_front(notification);
        Some(id)
    }

    pub fn set_max_history(&mut self, max_history: usize) {
        self.max_history = max_history;
        self.history.truncate(max_history);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(app: &str, text: &str) -> Request {
        Request { app_name: app.into(), summary: text.into(), ..Request::default() }
    }
    #[test]
    fn ids_are_nonzero_and_active_newest_first() {
        let mut store=NotificationStore::new(5);
        let a=store.push(req("a","first"));
        let b=store.push(req("a","second"));
        assert_ne!(a,b);
        assert_eq!(store.active()[0].id,b);
        assert_ne!(store.active()[0].id,0);
    }
    #[test]
    fn replacement_by_id_preserves_id_and_slot() {
        let mut store=NotificationStore::new(5);
        let id=store.push(req("a","old"));
        let mut changed=req("b","new");changed.replaces_id=id;
        assert_eq!(store.push(changed),id);
        assert_eq!(store.active().len(),1);
        assert_eq!(store.active()[0].summary,"new");
    }
    #[test]
    fn tag_replacement_is_scoped_to_application() {
        let mut store=NotificationStore::new(5);
        let mut a=req("one","old");a.tag="progress".into();
        let first=store.push(a);
        let mut b=req("two","other app");b.tag="progress".into();
        assert_ne!(first,store.push(b));
        let mut c=req("one","new");c.tag="progress".into();
        assert_eq!(first,store.push(c));
        assert_eq!(store.active().len(),2);
    }
    #[test]
    fn history_is_bounded_and_restore_uses_newest() {
        let mut store=NotificationStore::new(2);
        let a=store.push(req("a","one"));
        let b=store.push(req("a","two"));
        let c=store.push(req("a","three"));
        for id in [a,b,c] { store.close(id,CloseReason::Dismissed,true,true).unwrap(); }
        assert_eq!(store.history().len(),2);
        assert_eq!(store.history()[0].id,c);
        assert_eq!(store.restore(),Some(c));
        assert_eq!(store.active()[0].id,c);
    }
    #[test]
    fn no_history_when_disabled() {
        let mut store=NotificationStore::new(3);
        let id=store.push(req("a","one"));
        store.close(id,CloseReason::ClosedByCall,false,true).unwrap();
        assert!(store.history().is_empty());
    }
    #[test]
    fn respects_sender_timeout_unless_ignored() {
        let n=Notification::from_request(7,Request{requested_timeout_ms:9500,..req("x","x")});
        assert_eq!(n.timeout_ms(7000,false),Some(9500));
        assert_eq!(n.timeout_ms(7000,true),Some(7000));
        let n=Notification::from_request(8,Request{requested_timeout_ms:0,..req("x","x")});
        assert_eq!(n.timeout_ms(7000,false),None);
    }
}
