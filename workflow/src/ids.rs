use uuid::Uuid;

/// Current epoch millis (copied from harness message.rs:97 — std::time form; NOT an SDK fn).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn short_uuid() -> String {
    Uuid::new_v4().simple().to_string()
}

/// The ONLY random id in the worker.
pub fn new_run_id() -> String {
    format!("r_{}", short_uuid())
}

/// node_uid: node_id for a normal node, "{node_id}#{i}" for a fanned item.
pub fn node_uid(node_id: &str, item: Option<u32>) -> String {
    match item {
        None => node_id.to_string(),
        Some(i) => format!("{}#{}", node_id, i),
    }
}

/// Uid of loop pass `k` of a node: `"{node_id}#it{k}"`, or `node_id` itself for
/// the first pass (`k == 0`).
pub fn iter_uid(node_id: &str, k: u32) -> String {
    match k {
        0 => node_id.to_string(),
        k => format!("{node_id}#it{k}"),
    }
}

/// Split a uid into its node part and loop pass: `"v#it2"` → `("v", 2)`; any other
/// uid (a plain id, a fanout item) is pass 0 of itself.
pub fn split_iteration(uid: &str) -> (&str, u32) {
    uid.rsplit_once("#it")
        .filter(|(_, k)| k.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|(id, k)| Some((id, k.parse().ok()?)))
        .unwrap_or((uid, 0))
}

/// Deterministic child session id (opaque idempotency handle; never parsed back).
pub fn child_session_id(run_id: &str, node_uid: &str) -> String {
    format!("wf_{}_{}", run_id, node_uid)
}

/// Session id of one node attempt (`wf_<run>_<uid>@r<attempt>`), also its
/// `harness::send` idempotency key.
pub fn attempt_session_id(run_id: &str, node_uid: &str, attempt: u32) -> String {
    child_session_id(run_id, &format!("{node_uid}@r{attempt}"))
}

/// Key within scope "workflow_node_result" for a node's stored result blob.
pub fn node_result_key(run_id: &str, node_uid: &str) -> String {
    format!("{}/{}", run_id, node_uid)
}

/// Key within scope "workflow_def" for a run's frozen definition.
pub fn def_key(run_id: &str) -> String {
    run_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_uid_without_item() {
        assert_eq!(node_uid("read", None), "read");
    }

    #[test]
    fn test_node_uid_with_item() {
        assert_eq!(node_uid("read", Some(3)), "read#3");
    }

    #[test]
    fn iter_uid_and_split_iteration_round_trip() {
        assert_eq!(iter_uid("v", 0), "v");
        assert_eq!(iter_uid("v", 3), "v#it3");
        for k in 0..=9 {
            assert_eq!(
                split_iteration(&iter_uid("kan-12-verify", k)),
                ("kan-12-verify", k)
            );
        }
        // A fanout item, or a node id that merely contains "it", is not a pass.
        assert_eq!(split_iteration("read#3"), ("read#3", 0));
        assert_eq!(split_iteration("edit"), ("edit", 0));
        assert_eq!(split_iteration("v#it"), ("v#it", 0));
        assert_eq!(split_iteration("v#itx"), ("v#itx", 0));
    }

    #[test]
    fn test_child_session_id() {
        assert_eq!(child_session_id("r_1", "read#0"), "wf_r_1_read#0");
    }

    #[test]
    fn test_child_session_id_distinct_per_item() {
        let sid1 = child_session_id("r_1", "read#0");
        let sid2 = child_session_id("r_1", "read#1");
        assert_ne!(sid1, sid2);
    }

    #[test]
    fn test_child_session_id_distinct_per_attempt_but_stable_within_attempt() {
        // The tick uses attempt_session_id(run, node_uid, attempt) as BOTH the
        // session_id AND the harness idempotency_key. That is the crash-resume
        // fail-safe: re-firing the SAME attempt (a tick replay after a crash) reuses
        // the SAME key so harness::send dedups and does not double-spawn, while a
        // genuine retry (attempt+1) gets a FRESH key so it spawns a new session.
        let r0a = child_session_id("r_1", "read#0@r0");
        let r0b = child_session_id("r_1", "read#0@r0");
        let r1 = child_session_id("r_1", "read#0@r1");
        assert_eq!(
            r0a, r0b,
            "same (run,node,attempt) must yield a stable idempotency key"
        );
        assert_ne!(
            r0a, r1,
            "a retry (attempt+1) must yield a fresh idempotency key"
        );
    }

    #[test]
    fn test_node_result_key() {
        assert_eq!(node_result_key("r_1", "read#0"), "r_1/read#0");
    }

    #[test]
    fn test_def_key() {
        assert_eq!(def_key("r_1"), "r_1");
    }
}
