//! Deterministic expiration scheduler for the future Wayland/D-Bus event loop.
//! A notification's timeout is computed by `Notification::timeout_ms`.
//!
//! Mako uses per-notification event loop timers; here we keep the same
//! replacement/cancellation semantics without binding to a particular runtime.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::notification::{CloseReason, NotificationStore};

#[derive(Clone, Debug, Default)]
pub struct ExpirationScheduler {
    // Mako retains its currently chosen timer across calls to
    // update_event_loop_timer(): a newly armed equal-deadline timer does
    // not displace it. After the chosen timer is destroyed, the remaining
    // timers are scanned newest-first, because wl_list_insert uses the head.
    deadlines: HashMap<u32, (Instant, u128)>,
    next_sequence: u128,
    selected: Option<u32>,
}

impl ExpirationScheduler {
    /// Reconsider the chosen Mako timer without replacing it on equal
    /// deadlines. When the previous chosen timer was destroyed, the
    /// newest insertion wins an exact tie during the new list traversal.
    fn select_next(&mut self) {
        if let Some(current)=self.selected {
            if let Some(&(at,_))=self.deadlines.get(&current) {
                if let Some((&id,_))=self.deadlines.iter()
                    .filter(|(_, (candidate, _))| *candidate < at)
                    .min_by_key(|(_, (candidate, serial))|
                        (*candidate, Reverse(*serial))) {
                    self.selected=Some(id);
                }
                return;
            }
        }
        self.selected=self.deadlines.iter()
            .min_by_key(|(_, (at, serial))|(*at, Reverse(*serial)))
            .map(|(&id,_)|id);
    }

    /// Replace the previous timer for this ID. None/zero cancels the timer.
    pub fn set(&mut self, id: u32, now: Instant, timeout_ms: Option<u32>) {
        self.cancel(id);
        if let Some(ms) = timeout_ms.filter(|value| *value != 0) {
            if let Some(deadline) = now.checked_add(Duration::from_millis(u64::from(ms))) {
                self.next_sequence = self.next_sequence.checked_add(1)
                    .expect("notification timer serial exhausted");
                self.deadlines.insert(id, (deadline, self.next_sequence));
                self.select_next();
            }
        }
    }

    pub fn cancel(&mut self, id: u32) -> bool {
        let removed=self.deadlines.remove(&id).is_some();
        if removed {
            if self.selected==Some(id) {self.selected=None;}
            self.select_next();
        }
        removed
    }

    pub fn deadline(&self, id: u32) -> Option<Instant> {
        self.deadlines.get(&id).map(|(deadline, _)| *deadline)
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.selected.and_then(|id|self.deadline(id))
    }

    /// Mirror event-loop.c: the selected timer fires first on an equal
    /// deadline; its removal then elects the newest remaining equal timer.
    /// The scheduler's bulk-expiry caller does not interleave callbacks.
    pub fn take_expired(&mut self, now: Instant) -> Vec<u32> {
        let mut ready=Vec::new();
        while let Some(id)=self.selected {
            if self.deadline(id).is_none_or(|deadline|deadline>now){
                break;
            }
            self.deadlines.remove(&id);
            self.selected=None;
            self.select_next();
            ready.push(id);
        }
        ready
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
        assert_eq!(t.take_expired(now + Duration::from_millis(100)), vec![3, 1]);
    }

    #[test]
    fn identical_deadlines_preserve_selected_then_choose_newest_remaining() {
        let now = Instant::now();
        let mut t = ExpirationScheduler::default();
        t.set(1, now, Some(100));
        t.set(500, now, Some(100));
        t.set(7, now, Some(100));
        assert_eq!(t.next_deadline(), Some(now + Duration::from_millis(100)));
        assert_eq!(t.take_expired(now + Duration::from_millis(99)), Vec::<u32>::new());
        assert_eq!(t.take_expired(now + Duration::from_millis(100)), vec![1, 7, 500]);
    }

    #[test]
    fn rearm_moves_an_existing_id_ahead_of_equal_deadline_peers() {
        let now = Instant::now();
        let mut t = ExpirationScheduler::default();
        t.set(1, now, Some(100));
        t.set(2, now, Some(100));
        // Replacing an existing timer destroys it and prepends a fresh one
        // in Mako, even when its new absolute deadline is unchanged.
        t.set(1, now, Some(100));
        assert_eq!(t.deadline(1), Some(now + Duration::from_millis(100)));
        assert_eq!(t.take_expired(now + Duration::from_millis(100)), vec![2, 1]);
        // Cancelled timers never participate in a deadline tie.
        t.set(8, now, Some(200));
        t.set(9, now, Some(200));
        assert!(t.cancel(9));
        assert_eq!(t.take_expired(now + Duration::from_millis(200)), vec![8]);
    }

    #[test]
    fn cancelling_selected_timer_reselects_newest_of_equal_remaining() {
        let now=Instant::now();
        let mut t=ExpirationScheduler::default();
        t.set(10,now,Some(100));
        t.set(20,now,Some(100));
        t.set(30,now,Some(100));
        assert!(t.cancel(10));
        assert_eq!(t.take_expired(now+Duration::from_millis(100)),vec![30,20]);
        // An earlier deadline preempts the currently selected timer.
        t.set(10,now,Some(300));
        t.set(20,now,Some(300));
        t.set(30,now,Some(200));
        assert_eq!(t.take_expired(now+Duration::from_millis(300)),vec![30,10,20]);
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
