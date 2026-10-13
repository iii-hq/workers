//! Prompt extraction + config loading tests.

use codex::config::Config;
use codex::functions::types::{extract_prompt, RunRequest};
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
    let cfg = Config::load("/nonexistent/config.yaml").unwrap();
    assert_eq!(cfg.defaults.sandbox_mode, "workspace-write");
    assert_eq!(cfg.defaults.approval_policy, "never");
    assert!(cfg.defaults.skip_git_repo_check);
    assert!(cfg.iii_context);
    assert_eq!(cfg.codex_executable, "");
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
    assert_eq!(back.defaults.sandbox_mode, cfg.defaults.sandbox_mode);
    assert_eq!(back.codex_executable, cfg.codex_executable);
    assert_eq!(back.iii_context, cfg.iii_context);
    // the published schema is a JSON-schema object
    assert_eq!(Config::json_schema()["type"], "object");
}

#[test]
fn config_merges_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "defaults:\n  sandbox_mode: read-only\n").unwrap();
    let cfg = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(cfg.defaults.sandbox_mode, "read-only");
    assert_eq!(cfg.defaults.approval_policy, "never"); // default retained
}

#[test]
fn retired_stream_keys_still_load_and_are_ignored() {
    // A config stored before the move to owned trigger types still carries the
    // stream names; it must keep loading (from the configuration worker and
    // from a seed file) and the keys are dropped on the next write.
    let stored = json!({
        "defaults": { "sandbox_mode": "read-only" },
        "events_stream": "agent::events",
        "raw_events_stream": "codex::events",
        "iii_context": false,
    });
    let cfg = Config::from_json(&stored).unwrap();
    assert_eq!(cfg.defaults.sandbox_mode, "read-only");
    assert!(!cfg.iii_context);
    let written = cfg.to_json();
    assert!(written.get("events_stream").is_none());
    assert!(written.get("raw_events_stream").is_none());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "events_stream: agent::events\nraw_events_stream: codex::events\n",
    )
    .unwrap();
    assert!(Config::load(path.to_str().unwrap()).is_ok());
}

#[test]
fn schema_and_manifest_carry_no_stream_keys() {
    let schema = Config::json_schema().to_string();
    assert!(!schema.contains("events_stream"), "{schema}");
    let manifest = serde_json::to_value(codex::manifest::build_manifest()).unwrap();
    let defaults = manifest["default_config"].as_object().unwrap();
    assert!(!defaults.contains_key("events_stream"));
    assert!(!defaults.contains_key("raw_events_stream"));
    assert!(!manifest["description"]
        .as_str()
        .unwrap()
        .contains("agent::events"));
    // the seed file shipped next to the binary parses and matches the defaults
    let seed = Config::load(concat!(env!("CARGO_MANIFEST_DIR"), "/config.yaml")).unwrap();
    assert_eq!(seed.to_json(), Config::default().to_json());
}
