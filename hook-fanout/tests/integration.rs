//! Smoke tests that run without an iii engine connection.

#[test]
fn function_id_uses_worker_namespace() {
    assert_eq!(hook_fanout::FUNCTION_ID, "hook-fanout::publish_collect");
}

#[test]
fn reply_stream_is_agent_scoped() {
    assert_eq!(hook_fanout::HOOK_REPLY_STREAM, "agent::hook_reply");
}
