use crate::types::{NodeCheckpoint, NodeState};

#[derive(Debug, PartialEq)]
pub enum TimeoutAction {
    StillWaiting,
    Refire { attempt: u32 },
    FailOut,
}

/// Decide what to do with a node checkpoint at sweep time. Pure.
pub fn timeout_action(
    cp: &NodeCheckpoint,
    default_timeout_ms: u64,
    max_retries: u32,
    now: i64,
) -> TimeoutAction {
    if cp.state != NodeState::Running {
        return TimeoutAction::StillWaiting;
    }
    let timeout = cp.pending_timeout_ms.unwrap_or(default_timeout_ms);
    let pending_at = cp.pending_at.unwrap_or(now);
    // Clamp to 0 before the unsigned cast: a backward wall-clock/NTP step can make
    // `now < pending_at`, and a negative i64 cast straight to u64 wraps to ~1.8e19,
    // which reads as instantly expired. Matches the `.max(0)` clamp in reconcile/sweep.
    let expired = now.saturating_sub(pending_at).max(0) as u64 >= timeout;
    if !expired {
        return TimeoutAction::StillWaiting;
    }
    if cp.retries < max_retries {
        TimeoutAction::Refire {
            attempt: cp.retries + 1,
        }
    } else {
        TimeoutAction::FailOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NodeCheckpoint, NodeState};

    fn running_at(pending_at: i64, timeout: Option<u64>, retries: u32) -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Running,
            session_id: Some("s".into()),
            turn_id: Some("t".into()),
            result_ref: None,
            result_error: None,
            pending_at: Some(pending_at),
            pending_timeout_ms: timeout,
            retries,
        }
    }

    #[test]
    fn not_expired_is_still_waiting() {
        // now - pending_at < timeout
        let cp = running_at(1_000, Some(10_000), 0);
        assert!(matches!(
            timeout_action(&cp, 30_000, 1, 5_000),
            TimeoutAction::StillWaiting
        ));
    }
    #[test]
    fn expired_under_budget_refires_with_next_attempt() {
        let cp = running_at(0, Some(1_000), 0); // expired
        assert!(matches!(
            timeout_action(&cp, 30_000, 1, 5_000),
            TimeoutAction::Refire { attempt: 1 }
        ));
    }
    #[test]
    fn expired_at_budget_fails_out() {
        let cp = running_at(0, Some(1_000), 1); // retries already == max
        assert!(matches!(
            timeout_action(&cp, 30_000, 1, 5_000),
            TimeoutAction::FailOut
        ));
    }
    #[test]
    fn only_running_nodes_are_considered() {
        let mut cp = running_at(0, Some(1_000), 0);
        cp.state = NodeState::Done;
        assert!(matches!(
            timeout_action(&cp, 30_000, 1, 5_000),
            TimeoutAction::StillWaiting
        ));
    }

    #[test]
    fn backward_clock_is_not_instantly_expired() {
        // now < pending_at (clock stepped back): must NOT read as expired.
        let cp = running_at(10_000, Some(1_000), 0);
        assert!(matches!(
            timeout_action(&cp, 30_000, 1, 5_000), // now=5_000 < pending_at=10_000
            TimeoutAction::StillWaiting
        ));
    }
}
