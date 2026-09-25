//! The phantom-tick ring.
//!
//! The ingest runs as a trace of its own — the queue step, the
//! `database::*` calls it makes — and those traces tick the very `trace`
//! trigger that feeds it. The engine already drops ticks for spans of
//! functions registered as a trace-trigger target, which covers the handler,
//! but not the work the handler queued: a `database::execute` span belongs to
//! `database`, ticks normally, and turns one ingest into two.
//!
//! Registration metadata does not close this either. `metadata.internal` is
//! about discovery; the engine stamps `iii.function.kind = internal` only for
//! its own built-ins, so a worker cannot mark its spans as plumbing. The ring
//! is therefore the mechanism, not a belt: every trace this worker creates is
//! remembered for long enough to recognise its own tick and drop it.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Enough for a burst of ingest jobs without growing unbounded.
const CAPACITY: usize = 4_096;
/// Long enough to outlive the tick's coalescing window and the queue hop by a
/// wide margin, short enough that a trace id is never held for a whole run.
const TTL: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub struct PhantomRing {
    seen: Mutex<VecDeque<(String, Instant)>>,
    capacity: usize,
    ttl: Duration,
}

impl Default for PhantomRing {
    fn default() -> Self {
        Self::new(CAPACITY, TTL)
    }
}

impl PhantomRing {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            seen: Mutex::new(VecDeque::with_capacity(capacity.min(1_024))),
            capacity,
            ttl,
        }
    }

    /// Remember a trace this worker is creating right now.
    pub fn remember(&self, trace_id: &str) {
        if trace_id.is_empty() {
            return;
        }
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        prune(&mut seen, now, self.ttl);
        if seen.iter().any(|(known, _)| known == trace_id) {
            return;
        }
        if seen.len() >= self.capacity {
            seen.pop_front();
        }
        seen.push_back((trace_id.to_string(), now));
    }

    /// Whether this trace is one of ours.
    pub fn contains(&self, trace_id: &str) -> bool {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        prune(&mut seen, now, self.ttl);
        seen.iter().any(|(known, _)| known == trace_id)
    }

    /// Split a tick into the ids worth queueing and the count dropped.
    pub fn filter<'a, I>(&self, trace_ids: I) -> (Vec<String>, u64)
    where
        I: IntoIterator<Item = &'a String>,
    {
        let mut kept = Vec::new();
        let mut dropped = 0;
        for trace_id in trace_ids {
            if self.contains(trace_id) {
                dropped += 1;
            } else {
                kept.push(trace_id.clone());
            }
        }
        (kept, dropped)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

fn prune(seen: &mut VecDeque<(String, Instant)>, now: Instant, ttl: Duration) {
    while let Some((_, at)) = seen.front() {
        if now.duration_since(*at) < ttl {
            break;
        }
        seen.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_this_worker_created_is_recognised_and_dropped() {
        let ring = PhantomRing::default();
        ring.remember("own-trace");

        let ids = vec!["own-trace".to_string(), "someone-elses".to_string()];
        let (kept, dropped) = ring.filter(&ids);
        assert_eq!(kept, vec!["someone-elses"]);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn remembering_the_same_trace_twice_costs_one_slot() {
        let ring = PhantomRing::default();
        ring.remember("t1");
        ring.remember("t1");
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn the_ring_forgets_the_oldest_rather_than_growing() {
        let ring = PhantomRing::new(2, TTL);
        ring.remember("a");
        ring.remember("b");
        ring.remember("c");
        assert_eq!(ring.len(), 2);
        assert!(!ring.contains("a"), "the oldest id makes room");
        assert!(ring.contains("c"));
    }

    #[test]
    fn an_entry_expires_so_a_trace_id_is_never_held_forever() {
        let ring = PhantomRing::new(16, Duration::from_millis(1));
        ring.remember("t1");
        assert!(ring.contains("t1"));
        std::thread::sleep(Duration::from_millis(5));
        assert!(!ring.contains("t1"), "the window closed");
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn an_empty_trace_id_is_not_remembered() {
        let ring = PhantomRing::default();
        ring.remember("");
        assert_eq!(ring.len(), 0);
    }
}
