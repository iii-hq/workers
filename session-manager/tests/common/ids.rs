//! Deterministic id + clock injection for @pure scenarios, so feature
//! files can assert exact ids (`s_001`, `e_002`, ...) and timestamps.

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use session_manager::service::{Clock, IdGen};

/// Sequential ids: sessions `s_001, s_002, ...`, entries `e_001, ...`.
#[derive(Default)]
pub struct SeqIds {
    sessions: AtomicU64,
    entries: AtomicU64,
}

impl IdGen for SeqIds {
    fn session_id(&self) -> String {
        format!("s_{:03}", self.sessions.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn entry_id(&self) -> String {
        format!("e_{:03}", self.entries.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

/// Manually-advanced clock. Starts at 1_000_000 ms.
pub struct FakeClock {
    now: AtomicI64,
}

impl FakeClock {
    pub const START: i64 = 1_000_000;

    pub fn new() -> Self {
        Self {
            now: AtomicI64::new(Self::START),
        }
    }

    pub fn set(&self, ms: i64) {
        self.now.store(ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}
