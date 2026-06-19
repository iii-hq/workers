use std::time::{Duration, Instant};

use crate::deps::RuntimeState;

pub fn should_edit(
    runtime: &RuntimeState,
    chat_id: i64,
    message_id: i64,
    revision: u64,
    session_id: &str,
    entry_id: &str,
    throttle_ms: u64,
) -> bool {
    let rev_key = (session_id.to_string(), entry_id.to_string());
    if let Some(prev) = runtime.revisions.get(&rev_key) {
        if revision <= *prev {
            return false;
        }
    }

    let edit_key = (chat_id, message_id);
    let now = Instant::now();
    if let Some(last) = runtime.edit_times.get(&edit_key) {
        if now.duration_since(*last) < Duration::from_millis(throttle_ms) {
            return false;
        }
    }

    runtime.revisions.insert(rev_key, revision);
    runtime.edit_times.insert(edit_key, now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::RuntimeState;

    #[test]
    fn revision_monotonicity() {
        let rt = RuntimeState::new();
        assert!(should_edit(&rt, 1, 10, 1, "s1", "e1", 0));
        assert!(!should_edit(&rt, 1, 10, 1, "s1", "e1", 0));
        assert!(should_edit(&rt, 1, 10, 2, "s1", "e1", 0));
    }

    #[test]
    fn throttle_blocks_rapid_edits() {
        let rt = RuntimeState::new();
        assert!(should_edit(&rt, 1, 10, 1, "s1", "e1", 60_000));
        assert!(!should_edit(&rt, 1, 10, 2, "s1", "e1", 60_000));
    }

    #[test]
    fn throttle_does_not_advance_revision_when_time_blocked() {
        let rt = RuntimeState::new();
        assert!(should_edit(&rt, 1, 10, 1, "s1", "e1", 60_000));
        assert!(!should_edit(&rt, 1, 10, 2, "s1", "e1", 60_000));
        // Revision 2 was throttled, not consumed — a later attempt with no throttle succeeds.
        assert!(should_edit(&rt, 1, 10, 2, "s1", "e1", 0));
        assert!(!should_edit(&rt, 1, 10, 2, "s1", "e1", 0));
    }
}
