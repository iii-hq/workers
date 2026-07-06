//! Unit tests for the pure request/config helpers — no engine required.

use devin::config::Config;
use devin::functions::types::{
    extract_prompt, Message, RunRequest, SessionCreateRequest, SessionListRequest,
};
use serde_json::json;

#[test]
fn default_config_targets_v1_personal_mode() {
    let cfg = Config::default();
    // Default is the flat v1 API (personal tokens); org_id empty selects it.
    assert_eq!(cfg.base_url, "https://api.devin.ai/v1");
    assert_eq!(cfg.org_id, "");
    assert_eq!(cfg.request_timeout_secs, 120);
    assert!(!cfg.iii_context);
    assert_eq!(cfg.devin_bin(), "devin");
}

#[test]
fn config_schema_serializes() {
    let schema = Config::json_schema();
    assert!(schema.is_object());
}

#[test]
fn load_expands_env_and_unset_org_is_empty() {
    std::env::set_var("DEVIN_TEST_KEY_XYZ", "secret123");
    std::env::remove_var("DEVIN_TEST_ORG_XYZ");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "api_key: \"${DEVIN_TEST_KEY_XYZ}\"\norg_id: \"${DEVIN_TEST_ORG_XYZ}\"\n",
    )
    .unwrap();
    let cfg = Config::load(path.to_str().unwrap()).unwrap();
    assert_eq!(cfg.api_key, "secret123");
    // An unset org var expands to empty, which selects the v1 (personal) mode.
    assert_eq!(cfg.org_id, "");
    std::env::remove_var("DEVIN_TEST_KEY_XYZ");
}

#[test]
fn create_body_includes_prompt_and_only_present_fields() {
    let req = SessionCreateRequest {
        prompt: Some("build the thing".into()),
        title: Some("my task".into()),
        tags: Some(vec!["ci".into(), "urgent".into()]),
        devin_mode: Some("fast".into()),
        repos: Some(vec!["owner/repo".into()]),
        ..Default::default()
    };
    let body = req.to_body("build the thing");
    assert_eq!(body["prompt"], json!("build the thing"));
    assert_eq!(body["title"], json!("my task"));
    assert_eq!(body["tags"], json!(["ci", "urgent"]));
    assert_eq!(body["devin_mode"], json!("fast"));
    assert_eq!(body["repos"], json!(["owner/repo"]));
    // Absent optional fields must not appear in the body.
    assert!(body.get("playbook_id").is_none());
    assert!(body.get("max_acu_limit").is_none());
    assert!(body.get("bypass_approval").is_none());
}

#[test]
fn list_query_omits_absent_fields() {
    let req = SessionListRequest {
        limit: Some(10),
        tags: Some(vec!["a".into(), "b".into()]),
        ..Default::default()
    };
    let q = req.to_query();
    assert_eq!(q["limit"], json!(10));
    assert_eq!(q["tags"], json!("a,b"));
    assert!(q.get("offset").is_none());
}

#[test]
fn extract_prompt_prefers_explicit_prompt() {
    let req = RunRequest {
        prompt: Some("hello".into()),
        ..Default::default()
    };
    assert_eq!(extract_prompt(&req).unwrap(), "hello");
}

#[test]
fn extract_prompt_falls_back_to_last_user_message() {
    let req = RunRequest {
        messages: Some(vec![
            Message {
                role: "assistant".into(),
                content: json!("ignore me"),
            },
            Message {
                role: "user".into(),
                content: json!("do this"),
            },
        ]),
        ..Default::default()
    };
    assert_eq!(extract_prompt(&req).unwrap(), "do this");
}

#[test]
fn extract_prompt_errors_when_nothing_supplied() {
    let req = RunRequest::default();
    assert!(extract_prompt(&req).is_err());
}
