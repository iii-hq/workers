//! Process-local memo of the negative tombstone check: "this session and its
//! durable ancestors carry no deletion guard".
//!
//! [`crate::functions::delete_session_tree::guard_owner`] walks the lineage
//! with one guard read and one metadata read per level, and regular session
//! work asks it on every step, every dispatched call, and every send, spawn
//! and wake. Tombstones are written only by this process's deletion paths —
//! the single-process ownership `architecture/session-tree-deletion.md`
//! already assumes — and each guard CAS calls [`LivenessMemo::invalidate`] as
//! soon as it returns. An entry is honoured only while the epoch it was
//! stamped with is still current, so no memoized "live" answer outlives a
//! tombstone write in this process. Only "live" is memoized: an owner is
//! always read from state. [`TTL`] bounds staleness against writers outside
//! this process, which the design does not support but should not trust
//! indefinitely.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Upper bound on how long a "live" answer is reused without re-reading state.
pub const TTL: Duration = Duration::from_secs(2);
/// Past this many entries, expired ones are pruned (all of them, if none has).
const MAX_ENTRIES: usize = 4_096;

/// Taken BEFORE the authoritative walk's first read; see
/// [`LivenessMemo::remember_live`].
#[derive(Debug, Clone, Copy)]
pub struct Stamp {
    epoch: u64,
    at: Instant,
}

#[derive(Clone, Default)]
pub struct LivenessMemo {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    epoch: AtomicU64,
    live: Mutex<HashMap<String, Stamp>>,
}

impl LivenessMemo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stamp a walk before it reads anything, so a tombstone written while
    /// the walk is in flight already invalidates what it will remember.
    pub fn stamp(&self) -> Stamp {
        Stamp {
            epoch: self.inner.epoch.load(Ordering::SeqCst),
            at: Instant::now(),
        }
    }

    /// `session_id` was found live at the current epoch, within [`TTL`].
    pub fn is_live(&self, session_id: &str) -> bool {
        let epoch = self.inner.epoch.load(Ordering::SeqCst);
        self.entries()
            .get(session_id)
            .is_some_and(|stamp| stamp.epoch == epoch && stamp.at.elapsed() < TTL)
    }

    /// Record a walk that found no guard. Dropped when any tombstone was
    /// written (or released) since `stamp` was taken.
    pub fn remember_live(&self, session_id: &str, stamp: Stamp) {
        let mut live = self.entries();
        if stamp.epoch != self.inner.epoch.load(Ordering::SeqCst) {
            return;
        }
        if live.len() >= MAX_ENTRIES {
            live.retain(|_, kept| kept.epoch == stamp.epoch && kept.at.elapsed() < TTL);
            if live.len() >= MAX_ENTRIES {
                live.clear();
            }
        }
        live.insert(session_id.to_string(), stamp);
    }

    /// A tombstone write or release has completed — or may have, when its
    /// RPC failed: forget every "live" answer. Call AFTER the write returns,
    /// never before it: a walk stamped in between would otherwise remember a
    /// pre-write read under the post-write epoch.
    pub fn invalidate(&self) {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.entries().clear();
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Stamp>> {
        // Every critical section is one map operation, so a poisoned lock
        // cannot hold a torn entry.
        self.inner
            .live
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_answer_is_reused_until_a_tombstone_write_invalidates_it() {
        let memo = LivenessMemo::new();
        assert!(!memo.is_live("s"));
        memo.remember_live("s", memo.stamp());
        assert!(memo.is_live("s"));
        assert!(!memo.is_live("other"));
        memo.invalidate();
        assert!(!memo.is_live("s"));
    }

    #[test]
    fn a_walk_stamped_before_a_tombstone_write_is_not_remembered() {
        let memo = LivenessMemo::new();
        let stamp = memo.stamp();
        // A guard CAS returns while the walk is still reading.
        memo.invalidate();
        memo.remember_live("s", stamp);
        assert!(!memo.is_live("s"));
        // A walk stamped after the write is remembered as usual.
        memo.remember_live("s", memo.stamp());
        assert!(memo.is_live("s"));
    }

    #[test]
    fn an_answer_older_than_the_ttl_is_not_reused() {
        let memo = LivenessMemo::new();
        let Some(at) = Instant::now().checked_sub(TTL + Duration::from_millis(1)) else {
            return; // Monotonic clock too close to its origin to go back.
        };
        let stale = Stamp {
            epoch: memo.stamp().epoch,
            at,
        };
        memo.remember_live("s", stale);
        assert!(!memo.is_live("s"));
    }

    #[test]
    fn the_memo_stays_bounded() {
        let memo = LivenessMemo::new();
        for i in 0..(MAX_ENTRIES + 10) {
            memo.remember_live(&format!("s{i}"), memo.stamp());
        }
        assert!(memo.entries().len() <= MAX_ENTRIES);
        assert!(memo.is_live(&format!("s{}", MAX_ENTRIES + 9)));
    }
}
