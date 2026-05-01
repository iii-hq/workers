//! Smoke tests that run without an iii engine connection.

#[test]
fn function_id_in_subagent_namespace() {
    assert!(shell_subagent::start::ID.starts_with("shell::subagent::"));
}

#[test]
fn description_is_non_empty() {
    assert!(!shell_subagent::start::DESCRIPTION.is_empty());
}
