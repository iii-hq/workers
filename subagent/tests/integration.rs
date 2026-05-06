//! Smoke tests that run without an iii engine connection.

#[test]
fn function_id_in_subagent_namespace() {
    assert!(subagent::start::ID.starts_with("subagent::"));
}

#[test]
fn description_is_non_empty() {
    assert!(!subagent::start::DESCRIPTION.is_empty());
}
