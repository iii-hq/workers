//! request_id → in-flight stream handle for router::abort. In-memory only:
//! an abort must reach the router instance holding the stream
//! (single-instance worker — design doc § router::abort).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type Closer = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Default)]
pub struct InflightEntry {
    pub aborted: Arc<AtomicBool>,
    closer: Arc<Mutex<Option<Closer>>>,
}

impl InflightEntry {
    /// chat.rs points this at the current attempt's provider-channel closer.
    pub fn set_closer(&self, closer: Arc<dyn Fn() + Send + Sync>) {
        *self.closer.lock().unwrap() = Some(closer);
    }
    pub fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
        if let Some(close) = self.closer.lock().unwrap().clone() {
            close();
        }
    }
}

#[derive(Default)]
pub struct InflightMap {
    entries: Mutex<HashMap<String, InflightEntry>>,
}

impl InflightMap {
    pub fn insert(&self, request_id: &str) -> InflightEntry {
        let entry = InflightEntry::default();
        self.entries
            .lock()
            .unwrap()
            .insert(request_id.to_string(), entry.clone());
        entry
    }
    pub fn remove(&self, request_id: &str) {
        self.entries.lock().unwrap().remove(request_id);
    }
    /// false when unknown or already terminal (spec: AbortResponse.aborted).
    pub fn abort(&self, request_id: &str) -> bool {
        let entry = self.entries.lock().unwrap().remove(request_id);
        match entry {
            Some(e) => {
                e.abort();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn abort_fires_once_sets_flag_and_runs_the_closer() {
        let map = InflightMap::default();
        let entry = map.insert("r1");
        let closed = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c2 = closed.clone();
        entry.set_closer(std::sync::Arc::new(move || {
            c2.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(map.abort("r1"));
        assert!(entry.aborted.load(Ordering::SeqCst));
        assert_eq!(closed.load(Ordering::SeqCst), 1);
        assert!(!map.abort("r1")); // already terminal
        map.insert("r2");
        map.remove("r2");
        assert!(!map.abort("r2"));
    }
}
