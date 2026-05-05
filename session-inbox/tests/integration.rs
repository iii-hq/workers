//! Smoke tests that run without an iii engine connection.

#[test]
fn function_ids_in_inbox_namespace() {
    assert_eq!(session_inbox::PUSH_ID, "inbox::push");
    assert_eq!(session_inbox::DRAIN_ID, "inbox::drain");
    assert_eq!(session_inbox::PEEK_ID, "inbox::peek");
}

#[test]
fn inbox_key_uses_session_namespace() {
    assert_eq!(
        session_inbox::inbox_key("steering", "s1"),
        "session/s1/steering"
    );
}
