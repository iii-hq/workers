//! Smoke tests that run without an iii engine connection.

#[test]
fn function_ids_match_namespace() {
    assert!(shell_bash::exec::ID.starts_with("shell::bash::"));
    assert!(shell_bash::which::ID.starts_with("shell::bash::"));
    assert!(shell_bash::detect_clis::ID.starts_with("shell::bash::"));
}

#[test]
fn detect_clis_probes_at_least_one_cli() {
    assert!(!shell_bash::detect_clis::PROBED.is_empty());
}
