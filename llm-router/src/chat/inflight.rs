//! request_id → in-flight stream handle for router::abort. In-memory only:
//! an abort must reach the router instance holding the stream
//! (single-instance worker — design doc § router::abort).
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type Closer = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct InflightEntry {
    pub aborted: Arc<AtomicBool>,
    closer: Arc<Mutex<Option<Closer>>>,
    abort_notify: Arc<tokio::sync::Notify>,
}

impl InflightEntry {
    /// chat.rs points this at the current attempt's provider-channel closer.
    pub fn set_closer(&self, closer: Arc<dyn Fn() + Send + Sync>) {
        let mut slot = self.closer.lock().unwrap();
        if self.aborted.load(Ordering::SeqCst) {
            drop(slot);
            closer();
        } else {
            *slot = Some(closer);
        }
    }
    pub fn abort(&self) -> bool {
        if self.aborted.swap(true, Ordering::SeqCst) {
            return false;
        }
        // A stored permit wakes a retry backoff even when abort races the
        // waiter being installed.
        self.abort_notify.notify_one();
        if let Some(close) = self.closer.lock().unwrap().clone() {
            close();
        }
        true
    }

    /// Wait for the retry delay while remaining immediately abortable.
    /// Returns true when cancellation won (including a cancellation that
    /// landed before this method was called).
    pub async fn wait_for_abort(&self, delay: std::time::Duration) -> bool {
        if self.aborted.load(Ordering::SeqCst) {
            return true;
        }
        tokio::select! {
            _ = self.abort_notify.notified() => true,
            _ = tokio::time::sleep(delay) => self.aborted.load(Ordering::SeqCst),
        }
    }
}

#[derive(Default)]
pub struct InflightMap {
    entries: Mutex<HashMap<String, InflightEntry>>,
}

/// Owns one request-id reservation. Dropping the chat future releases only
/// this exact entry, so cancellation cannot leak an id or remove a newer call.
pub struct InflightReservation {
    map: Arc<InflightMap>,
    request_id: String,
    entry: InflightEntry,
}

impl Deref for InflightReservation {
    type Target = InflightEntry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl Drop for InflightReservation {
    fn drop(&mut self) {
        self.map.remove_if_same(&self.request_id, &self.entry);
    }
}

impl InflightMap {
    /// Atomically reserves a request id. A live request keeps the reservation
    /// after `router::abort` until its chat future actually terminates.
    pub fn reserve(self: &Arc<Self>, request_id: &str) -> Option<InflightReservation> {
        let entry = InflightEntry::default();
        let mut entries = self.entries.lock().unwrap();
        if entries.contains_key(request_id) {
            return None;
        }
        entries.insert(request_id.to_string(), entry.clone());
        Some(InflightReservation {
            map: self.clone(),
            request_id: request_id.to_string(),
            entry,
        })
    }

    fn remove_if_same(&self, request_id: &str, expected: &InflightEntry) {
        let mut entries = self.entries.lock().unwrap();
        if entries
            .get(request_id)
            .is_some_and(|current| Arc::ptr_eq(&current.aborted, &expected.aborted))
        {
            entries.remove(request_id);
        }
    }

    /// False when unknown, already aborted, or terminal
    /// (spec: AbortResponse.aborted).
    pub fn abort(&self, request_id: &str) -> bool {
        let entry = self.entries.lock().unwrap().get(request_id).cloned();
        entry.is_some_and(|entry| entry.abort())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn abort_fires_once_sets_flag_and_runs_the_closer() {
        let map = Arc::new(InflightMap::default());
        let entry = map.reserve("r1").unwrap();
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c2 = closed.clone();
        entry.set_closer(std::sync::Arc::new(move || {
            c2.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(map.abort("r1"));
        assert!(entry.aborted.load(Ordering::SeqCst));
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        assert!(!map.abort("r1")); // already aborted
        let r2 = map.reserve("r2").unwrap();
        drop(r2); // terminal chat releases the reservation
        assert!(!map.abort("r2"));
    }

    #[test]
    fn duplicate_reservation_is_rejected_without_orphaning_the_first() {
        let map = Arc::new(InflightMap::default());
        let first = map.reserve("same-id").unwrap();

        assert!(map.reserve("same-id").is_none());
        assert!(map.abort("same-id"), "the original remains abortable");
        assert!(first.aborted.load(Ordering::SeqCst));
        assert!(
            map.reserve("same-id").is_none(),
            "abort does not release an id while its chat is still winding down"
        );

        drop(first);
        assert!(map.reserve("same-id").is_some());
    }

    #[test]
    fn closer_installed_after_abort_is_closed_immediately() {
        let map = Arc::new(InflightMap::default());
        let entry = map.reserve("r1").unwrap();
        assert!(map.abort("r1"));

        let closed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let observed = closed.clone();
        entry.set_closer(Arc::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        }));

        assert_eq!(closed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn abort_wakes_a_retry_delay_immediately() {
        let map = Arc::new(InflightMap::default());
        let entry = map.reserve("r1").unwrap();
        let waiter = {
            let entry = entry.clone();
            tokio::spawn(async move {
                entry
                    .wait_for_abort(std::time::Duration::from_secs(60))
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(map.abort("r1"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), waiter)
                .await
                .expect("abort must wake the retry waiter")
                .expect("waiter task joins")
        );
    }
}
