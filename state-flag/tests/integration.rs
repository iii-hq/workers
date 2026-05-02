//! Smoke tests that run without an iii engine connection.

#[test]
fn function_ids_in_flag_namespace() {
    assert_eq!(state_flag::SET_ID, "flag::set");
    assert_eq!(state_flag::CLEAR_ID, "flag::clear");
    assert_eq!(state_flag::IS_SET_ID, "flag::is_set");
}

#[test]
fn flag_key_uses_session_namespace() {
    assert!(state_flag::flag_key("paused", "s1").contains("s1"));
    assert!(state_flag::flag_key("paused", "s1").contains("paused"));
}
