use crate::types::{NodeCheckpoint, NodeState, WorkflowDef};

#[derive(Debug, PartialEq)]
pub enum TimeoutAction {
    StillWaiting,
    Refire { attempt: u32 },
    FailOut,
}

/// A node's `(timeout_ms, retries)`: its own values, else the worker defaults.
/// A fanout item `<id>#<i>` or a loop pass `<id>#it<k>` uses its base node's. Pure.
pub fn node_limits(
    def: &WorkflowDef,
    uid: &str,
    default_timeout_ms: u64,
    default_retries: u32,
) -> (u64, u32) {
    let node = uid.split('#').next().and_then(|id| def.nodes.get(id));
    (
        node.and_then(|n| n.timeout_ms)
            .unwrap_or(default_timeout_ms),
        node.and_then(|n| n.retries).unwrap_or(default_retries),
    )
}

/// Decide what to do with a node checkpoint at sweep time. Pure.
pub fn timeout_action(
    cp: &NodeCheckpoint,
    timeout_ms: u64,
    max_retries: u32,
    now: i64,
) -> TimeoutAction {
    if cp.state != NodeState::Running {
        return TimeoutAction::StillWaiting;
    }
    let pending_at = cp.pending_at.unwrap_or(now);
    // Clamp to 0 before the unsigned cast: a backward wall-clock/NTP step can make
    // `now < pending_at`, and a negative i64 cast straight to u64 wraps to ~1.8e19,
    // which reads as instantly expired. Matches the `.max(0)` clamp in reconcile/sweep.
    let expired = now.saturating_sub(pending_at).max(0) as u64 >= timeout_ms;
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

/// Sweep refire: the node goes back to Pending with the next attempt and keeps
/// the old session id, so the next tick fires it through `decide` in a new
/// `@r<attempt>` session and stops the old one.
pub fn mark_refire(cp: &mut NodeCheckpoint, attempt: u32) {
    cp.state = NodeState::Pending;
    cp.retries = attempt;
}

/// A send the harness refused outright; retrying cannot help. Matched with
/// `contains` because the SDK wraps the code (`remote error (<code>): harness/…`).
pub fn is_permanent_send_error(err: &str) -> bool {
    err.contains("harness/invalid_request")
}

/// Record a failed `harness::send` (or probe) on a reserved node. The node stays
/// Pending for a retry on the next tick unless the error is permanent or
/// `timeout_ms` has passed since the attempt's first failure.
pub fn apply_send_failure(cp: &mut NodeCheckpoint, err: &str, timeout_ms: u64, now: i64) {
    let first = *cp.send_failed_at.get_or_insert(now);
    cp.result_error = Some(format!("send failed: {err}"));
    let expired = now.saturating_sub(first).max(0) as u64 >= timeout_ms;
    if is_permanent_send_error(err) || expired {
        cp.state = NodeState::Failed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NodeCheckpoint, NodeState};

    fn running_at(pending_at: i64, retries: u32) -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Running,
            session_id: Some("s".into()),
            turn_id: Some("t".into()),
            pending_at: Some(pending_at),
            retries,
            ..Default::default()
        }
    }

    #[test]
    fn not_expired_is_still_waiting() {
        // now - pending_at < timeout
        let cp = running_at(1_000, 0);
        assert!(matches!(
            timeout_action(&cp, 10_000, 1, 5_000),
            TimeoutAction::StillWaiting
        ));
    }
    #[test]
    fn expired_under_budget_refires_with_next_attempt() {
        let cp = running_at(0, 0); // expired
        assert!(matches!(
            timeout_action(&cp, 1_000, 1, 5_000),
            TimeoutAction::Refire { attempt: 1 }
        ));
    }
    #[test]
    fn expired_at_budget_fails_out() {
        let cp = running_at(0, 1); // retries already == max
        assert!(matches!(
            timeout_action(&cp, 1_000, 1, 5_000),
            TimeoutAction::FailOut
        ));
    }
    #[test]
    fn only_running_nodes_are_considered() {
        let mut cp = running_at(0, 0);
        cp.state = NodeState::Done;
        assert!(matches!(
            timeout_action(&cp, 1_000, 1, 5_000),
            TimeoutAction::StillWaiting
        ));
    }
    #[test]
    fn retries_zero_fails_out_without_refire() {
        let cp = running_at(0, 0);
        assert_eq!(timeout_action(&cp, 1_000, 0, 5_000), TimeoutAction::FailOut);
    }

    #[test]
    fn node_limits_prefers_node_over_config() {
        use crate::types::{FanoutSpec, NodeDef, OutputRef};
        let node = |timeout_ms, retries, fanout: bool| NodeDef {
            timeout_ms,
            retries,
            fanout: fanout.then(|| FanoutSpec {
                over: "node:plan.result.docs".into(),
                items: None,
            }),
            ..Default::default()
        };
        let def = WorkflowDef {
            version: 1,
            nodes: [
                ("plan".to_string(), node(None, None, false)),
                ("read".to_string(), node(Some(120_000), Some(0), true)),
                ("half".to_string(), node(None, Some(3), false)),
            ]
            .into(),
            output: OutputRef {
                from: "node:plan".into(),
            },
            default_functions: None,
            max_concurrency: None,
        };
        assert_eq!(node_limits(&def, "plan", 30_000, 1), (30_000, 1));
        assert_eq!(node_limits(&def, "read#3", 30_000, 1), (120_000, 0));
        assert_eq!(node_limits(&def, "half", 30_000, 1), (30_000, 3));
        assert_eq!(node_limits(&def, "gone", 30_000, 1), (30_000, 1));
    }

    #[test]
    fn node_limits_for_pass_uid_use_base_node() {
        let def: WorkflowDef = serde_json::from_value(serde_json::json!({
            "nodes": {"v": {"agent": {"model": "m"}, "input": {"from": "run_input"},
                            "timeout_ms": 120_000, "retries": 0}},
            "output": {"from": "node:v"}
        }))
        .unwrap();
        assert_eq!(node_limits(&def, "v#it1", 30_000, 1), (120_000, 0));
        assert_eq!(node_limits(&def, "v#it9", 30_000, 1), (120_000, 0));
    }

    fn reserved() -> NodeCheckpoint {
        NodeCheckpoint {
            session_id: Some("wf_r_a@r0".into()),
            ..Default::default()
        }
    }

    #[test]
    fn send_failure_fails_fast_on_invalid_request() {
        let mut cp = reserved();
        let err = "call error: remote error (invocation_failed): harness/invalid_request: \
                   unknown agent profile 'no-such-profile'";
        apply_send_failure(&mut cp, err, 60_000, 1_000);
        assert_eq!(cp.state, NodeState::Failed);
        assert_eq!(
            cp.result_error.as_deref(),
            Some(&*format!("send failed: {err}"))
        );
    }

    #[test]
    fn send_failure_stays_pending_within_timeout() {
        let mut cp = reserved();
        apply_send_failure(&mut cp, "invocation timed out", 60_000, 1_000);
        apply_send_failure(&mut cp, "iii is not connected", 60_000, 60_999);
        assert_eq!(cp.state, NodeState::Pending);
        assert_eq!(
            cp.result_error.as_deref(),
            Some("send failed: iii is not connected")
        );
    }

    #[test]
    fn send_failure_fails_after_timeout() {
        let mut cp = reserved();
        apply_send_failure(&mut cp, "invocation timed out", 60_000, 1_000);
        apply_send_failure(&mut cp, "invocation timed out", 60_000, 61_000);
        assert_eq!(cp.state, NodeState::Failed);
    }

    #[test]
    fn send_failure_keeps_first_failed_at() {
        let mut cp = reserved();
        apply_send_failure(&mut cp, "invocation timed out", 60_000, 1_000);
        apply_send_failure(&mut cp, "invocation timed out", 60_000, 30_000);
        assert_eq!(cp.send_failed_at, Some(1_000));
    }

    #[test]
    fn backward_clock_is_not_instantly_expired() {
        // now < pending_at (clock stepped back): must NOT read as expired.
        let cp = running_at(10_000, 0);
        assert!(matches!(
            timeout_action(&cp, 1_000, 1, 5_000), // now=5_000 < pending_at=10_000
            TimeoutAction::StillWaiting
        ));
    }
}
