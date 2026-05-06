//! Smoke tests that run without an iii engine connection.

#[test]
fn state_keys_namespace_by_session() {
    let s = "sess-1";
    assert!(turn_orchestrator::messages_key(s).contains(s));
    assert!(turn_orchestrator::turn_state_key(s).contains(s));
    assert!(turn_orchestrator::run_request_key(s).contains(s));
}

#[test]
fn state_keys_distinct_per_facet() {
    let s = "sess-1";
    let keys = [
        turn_orchestrator::messages_key(s),
        turn_orchestrator::turn_state_key(s),
        turn_orchestrator::run_request_key(s),
        turn_orchestrator::cwd_key(s),
        turn_orchestrator::sandbox_id_key(s),
        turn_orchestrator::tool_schemas_key(s),
    ];
    let unique: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(unique.len(), keys.len(), "every facet has a distinct key");
}
