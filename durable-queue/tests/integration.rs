//! Smoke tests that run without an iii engine connection.

#[test]
fn function_ids_in_queue_namespace() {
    assert_eq!(durable_queue::PUSH_ID, "queue::push");
    assert_eq!(durable_queue::DRAIN_ID, "queue::drain");
    assert_eq!(durable_queue::PEEK_ID, "queue::peek");
}

#[test]
fn queue_key_uses_session_namespace() {
    assert_eq!(
        durable_queue::queue_key("steering", "s1"),
        "session/s1/steering"
    );
}
