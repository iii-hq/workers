//! Smoke tests that run without an iii engine connection.

#[test]
fn stream_constants_are_namespaced() {
    assert!(harness_runtime::EVENTS_STREAM.starts_with("agent::"));
    assert!(harness_runtime::HOOK_REPLY_STREAM.starts_with("agent::"));
}

#[test]
fn topics_are_distinct() {
    assert_ne!(harness_runtime::TOPIC_BEFORE, harness_runtime::TOPIC_AFTER);
}

#[test]
fn cwd_hash_is_stable_for_same_path() {
    use std::path::PathBuf;
    let a = harness_runtime::resume::cwd_hash(&PathBuf::from("/tmp/example"));
    let b = harness_runtime::resume::cwd_hash(&PathBuf::from("/tmp/example"));
    assert_eq!(a, b);
}
