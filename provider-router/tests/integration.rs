//! Smoke tests that run without an iii engine connection.

#[test]
fn stream_constants_are_namespaced() {
    assert!(provider_router::EVENTS_STREAM.starts_with("agent::"));
    assert!(provider_router::HOOK_REPLY_STREAM.starts_with("agent::"));
}

#[test]
fn topics_are_distinct() {
    assert_ne!(provider_router::TOPIC_BEFORE, provider_router::TOPIC_AFTER);
}

#[test]
fn cwd_hash_is_stable_for_same_path() {
    use std::path::PathBuf;
    let a = provider_router::resume::cwd_hash(&PathBuf::from("/tmp/example"));
    let b = provider_router::resume::cwd_hash(&PathBuf::from("/tmp/example"));
    assert_eq!(a, b);
}
