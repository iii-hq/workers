//! Agents never call context-clm directly: the harness runs the strategy on
//! their behalf, and status and the lifecycle sinks are operator/engine plumbing.

#[test]
fn every_context_clm_function_is_denied_to_agents() {
    for (file, text) in [
        (
            "context-clm/iii-permissions.yaml",
            include_str!("../iii-permissions.yaml"),
        ),
        (
            "iii-permissions.yaml",
            include_str!("../../iii-permissions.yaml"),
        ),
    ] {
        assert!(
            text.lines()
                .any(|line| line.trim() == "- '!context-clm::*'"),
            "{file} must deny context-clm::*"
        );
    }
}
