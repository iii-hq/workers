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
fn config_defaults_when_file_missing() {
    let cfg = Config::load("/nonexistent/config.yaml").unwrap();
    assert_eq!(cfg.engine_url, "ws://127.0.0.1:49134");
    assert_eq!(cfg.defaults.sandbox_mode, "workspace-write");
    assert_eq!(cfg.defaults.approval_policy, "never");
    assert!(cfg.defaults.skip_git_repo_check);
    assert_eq!(cfg.raw_events_stream, "codex::events");
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
fn config_merges_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(&path, "defaults:\n  sandbox_mode: read-only\n").unwrap();
    let cfg = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(cfg.defaults.sandbox_mode, "read-only");
    assert_eq!(cfg.defaults.approval_policy, "never"); // default retained
}
