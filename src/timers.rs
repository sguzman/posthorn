//! Deterministic expiration scheduler for the future Wayland/D-Bus event loop.
//! A notification's timeout is computed by `Notification::timeout_ms`.
//!
//! Mako uses per-notification event loop timers; here we keep the same
//! replacement/cancellation semantics without binding to a particular runtime.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::notification::{CloseReason, NotificationStore};

#[derive(Clone, Debug, Default)]
pub struct ExpirationScheduler {
    deadlines: HashMap<u32, Instant>,
}

impl ExpirationScheduler {
    /// Replace the previous timer for this ID. None/zero cancels the timer.
    pub fn set(&mut self, id: u32, now: Instant, timeout_ms: Option<u32>) {
        self.deadlines.remove(&id);
        if let Some(ms) = timeout_ms.filter(|value| *value != 0) {
            if let Some(deadline) = now.checked_add(Duration::from_millis(u64::from(ms))) {
                self.deadlines.insert(id, deadline);
            }
        }
    }

    pub fn cancel(&mut self, id: u32) -> bool {
        self.deadlines.remove(&id).is_some()
    }

    pub fn deadline(&self, id: u32) -> Option<Instant> {
        self.deadlines.get(&id).copied()
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.values().copied().min()
    }

    /// Remove and return expired IDs in deadline order, stable by ID for ties.
    pub fn take_expired(&mut self, now: Instant) -> Vec<u32> {
        let mut ready: Vec<_> = self.deadlines.iter()
            .filter(|(_, at)| **at <= now)
            .map(|(&id, &at)| (at, id)).collect();
        ready.sort_unstable();
        for &(_, id) in &ready {
            self.deadlines.remove(&id);
        }
        ready.into_iter().map(|(_, id)| id).collect()
    }

    /// Close due active notifications; the caller must emit NotificationClosed
    /// for returned IDs with reason Expired and request a Wayland redraw.
    pub fn expire_active(
        &mut self,
        now: Instant,
        store: &mut NotificationStore,
        history_enabled: bool,
    ) -> Vec<u32> {
        let mut closed = Vec::new();
        for id in self.take_expired(now) {
            if store.close(id, CloseReason::Expired, true, history_enabled).is_some() {
                closed.push(id);
            }
        }
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::Request;

    #[test]
    fn replacement_reschedules_without_old_timer_firing() {
        let at = Instant::now();
        let mut t = ExpirationScheduler::default();
        t.set(7, at, Some(1000));
        t.set(7, at + Duration::from_millis(500), Some(1200));
        assert!(t.take_expired(at + Duration::from_millis(1000)).is_empty());
        assert_eq!(t.take_expired(at + Duration::from_millis(1700)), vec![7]);
    }

    #[test]
    fn persistent_notification_has_no_timer() {
        let now = Instant::now();
        let mut t = ExpirationScheduler::default();
        t.set(3, now, Some(800));
        t.set(3, now, None);
        assert!(t.next_deadline().is_none());
        assert!(t.take_expired(now + Duration::from_secs(5)).is_empty());
    }

    #[test]
    fn deadlines_are_sorted_and_do_not_fire_early() {
        let now = Instant::now();
        let mut t = ExpirationScheduler::default();
        t.set(3, now, Some(100));
        t.set(1, now, Some(100));
        t.set(2, now, Some(50));
        assert!(t.take_expired(now + Duration::from_millis(49)).is_empty());
        assert_eq!(t.take_expired(now + Duration::from_millis(50)), vec![2]);
        assert_eq!(t.take_expired(now + Duration::from_millis(100)), vec![1, 3]);
    }

    #[test]
    fn expiration_closes_with_history_and_ignores_already_dismissed() {
        let now = Instant::now();
        let mut store = NotificationStore::new(5);
        let gone = store.push(Request::default());
        let live = store.push(Request::default());
        let mut t = ExpirationScheduler::default();
        t.set(gone, now, Some(100));
        t.set(live, now, Some(100));
        store.close(gone, CloseReason::Dismissed, true, true);
        assert_eq!(t.expire_active(now + Duration::from_millis(100), &mut store, true), vec![live]);
        assert_eq!(store.history().len(), 2);
        assert!(store.active().is_empty());
    }
}
