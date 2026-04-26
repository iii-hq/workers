//! Local-invocation → remote-task bookkeeping.
//!
//! Today this is a thin in-memory map of `local_invocation_id` →
//! `remote_task_id`. The intent is to let a future hook forward
//! local-side cancellation to `tasks/cancel` on the remote agent.
//!
//! iii-sdk 0.11.3 exposes no public hook for "this invocation was
//! cancelled by the engine", so the wiring is deferred. The map and the
//! API are here so adding the hook later is a one-file change. See
//! `README.md` → "Limitations" for the full picture.

use dashmap::DashMap;

#[derive(Debug, Default)]
pub struct TaskTracker {
    map: DashMap<String, String>,
}

impl TaskTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `local_id → remote_id`. `local_id` will eventually be the
    /// invocation id supplied by the engine.
    pub fn link(&self, local_id: impl Into<String>, remote_id: impl Into<String>) {
        self.map.insert(local_id.into(), remote_id.into());
    }

    pub fn remote_for(&self, local_id: &str) -> Option<String> {
        self.map.get(local_id).map(|v| v.clone())
    }

    pub fn forget(&self, local_id: &str) -> Option<String> {
        self.map.remove(local_id).map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let t = TaskTracker::new();
        t.link("inv-1", "task-1");
        assert_eq!(t.remote_for("inv-1").as_deref(), Some("task-1"));
        assert_eq!(t.forget("inv-1").as_deref(), Some("task-1"));
        assert!(t.remote_for("inv-1").is_none());
        assert!(t.is_empty());
    }
}
