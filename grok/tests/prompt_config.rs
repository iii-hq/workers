//! Prompt extraction + config loading tests.

use grok::config::Config;
use grok::functions::types::{extract_prompt, RunRequest};
use serde_json::json;

#[test]
fn explicit_prompt_wins_including_empty() {
    let req = RunRequest {
        prompt: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(extract_prompt(&req).unwrap(), "");
}

#[test]
fn extracts_last_user_message_text() {
    let req: RunRequest = serde_json::from_value(json!({
        "messages": [
            { "role": "user", "content": [{ "type": "text", "text": "first" }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "reply" }] },
            { "role": "user", "content": [{ "type": "text", "text": "second" }] }
        ]
    }))
    .unwrap();
    assert_eq!(extract_prompt(&req).unwrap(), "second");
}

#[test]
fn plain_string_message_content() {
    let req: RunRequest = serde_json::from_value(json!({
        "messages": [{ "role": "user", "content": "plain" }]
    }))
    .unwrap();
    assert_eq!(extract_prompt(&req).unwrap(), "plain");
}

#[test]
fn no_prompt_no_user_message_errors() {
    let req: RunRequest = serde_json::from_value(json!({
        "messages": [{ "role": "assistant", "content": "x" }]
    }))
    .unwrap();
    assert!(extract_prompt(&req).is_err());
}

#[test]
fn unsupported_message_content_errors() {
    let req: RunRequest = serde_json::from_value(json!({
        "messages": [{ "role": "user", "content": { "unexpected": "object" } }]
    }))
    .unwrap();
    assert!(extract_prompt(&req).is_err());
}

#[test]
fn config_defaults_when_file_missing() {
    // Guaranteed-missing path inside a temp dir (portable; no host assumptions).
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist.yaml");
    let cfg = Config::load(missing.to_str().unwrap()).unwrap();
    assert_eq!(cfg.defaults.model, "");
    assert!(cfg.defaults.always_approve);
    assert_eq!(cfg.raw_events_stream, "grok::events");
    assert!(cfg.iii_context);
    assert_eq!(cfg.grok_executable, "");
}

#[test]
fn config_parse_error_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "defaults: [unclosed\n  bad: {").unwrap();
    assert!(Config::load(path.to_str().unwrap()).is_err());
}

#[test]
fn config_round_trips_through_json_for_the_configuration_worker() {
    // to_json (initial_value) -> from_json (fetched value) preserves the config
    let cfg = Config::default();
    let json = cfg.to_json();
    let back = Config::from_json(&json).unwrap();
    assert_eq!(back.defaults.always_approve, cfg.defaults.always_approve);
    assert_eq!(back.raw_events_stream, cfg.raw_events_stream);
    assert_eq!(back.iii_context, cfg.iii_context);
    // the published schema is a JSON-schema object
    assert_eq!(Config::json_schema()["type"], "object");
}

#[test]
fn config_merges_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "defaults:\n  model: grok-build-0.1\n").unwrap();
    let cfg = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(cfg.defaults.model, "grok-build-0.1");
    assert!(cfg.defaults.always_approve); // default retained
}
