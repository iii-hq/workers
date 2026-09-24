// Shared by worker integration tests. Each case gets a fresh process because
// the configuration identity is deliberately cached for the process lifetime.
/// Check named and standalone identities in separate processes to avoid cached-env interference.
#[test]
fn configuration_identity_follows_compose_and_keeps_standalone_fallback() {
    for (input, expected) in [
        (None, DEFAULT_ID),
        (Some(""), DEFAULT_ID),
        (Some(" \t "), DEFAULT_ID),
        (
            Some("  project-worker-0123456789abcdef  "),
            "project-worker-0123456789abcdef",
        ),
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "configuration_identity_child", "--ignored"])
            .env_remove("III_CONFIG_NAME")
            .env("EXPECTED_CONFIG_ID", expected);
        if let Some(input) = input {
            child.env("III_CONFIG_NAME", input);
        }
        let output = child.output().expect("spawn isolated configuration test");
        assert!(
            output.status.success(),
            "{input:?}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Subprocess fixture reads the parent's expected ID after configuration environment setup.
#[test]
#[ignore = "subprocess fixture, invoked by the parent test"]
fn configuration_identity_child() {
    let expected = std::env::var("EXPECTED_CONFIG_ID").expect("parent supplies expectation");
    assert_eq!(config_id(), expected);
    assert_eq!(config_id(), config_id());
}
