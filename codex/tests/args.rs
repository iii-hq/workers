//! Arg + config serialization tests.

use codex::codex::args::{build_args, resolve, toml_value, ResolvedOptions};
use codex::config::Config;
use codex::functions::types::RunRequest;
use serde_json::{json, Map, Value};

fn opts() -> ResolvedOptions {
    ResolvedOptions {
        model: String::new(),
        cwd: String::new(),
        sandbox_mode: "workspace-write".into(),
        approval_policy: "never".into(),
        reasoning_effort: String::new(),
        skip_git_repo_check: true,
        base_url: String::new(),
        config: Map::new(),
        images: vec![],
        additional_directories: vec![],
    }
}

#[test]
fn builds_base_exec_json_invocation() {
    let a = build_args(&opts(), None, None);
    assert_eq!(a[0], "exec");
    assert!(a.contains(&"--json".to_string()));
    assert!(a.contains(&"--sandbox".to_string()));
    assert!(a.contains(&"workspace-write".to_string()));
    assert!(a.contains(&"--skip-git-repo-check".to_string()));
    // approval_policy goes through --config
    assert!(a.iter().any(|s| s.starts_with("approval_policy=")));
}

#[test]
fn resume_appends_subcommand() {
    let a = build_args(&opts(), Some("th-1"), None);
    let i = a
        .iter()
        .position(|s| s == "resume")
        .expect("resume present");
    assert_eq!(a[i + 1], "th-1");
}

#[test]
fn output_schema_path_passed() {
    let a = build_args(&opts(), None, Some("/tmp/schema.json"));
    let i = a.iter().position(|s| s == "--output-schema").unwrap();
    assert_eq!(a[i + 1], "/tmp/schema.json");
}

#[test]
fn model_cwd_images_mapped() {
    let mut o = opts();
    o.model = "gpt-5.2-codex".into();
    o.cwd = "/repo".into();
    o.images = vec!["/tmp/a.png".into()];
    let a = build_args(&o, None, None);
    let m = a.iter().position(|s| s == "--model").unwrap();
    assert_eq!(a[m + 1], "gpt-5.2-codex");
    let c = a.iter().position(|s| s == "--cd").unwrap();
    assert_eq!(a[c + 1], "/repo");
    let img = a.iter().position(|s| s == "--image").unwrap();
    assert_eq!(a[img + 1], "/tmp/a.png");
}

#[test]
fn toml_value_quotes_and_escapes_multiline_strings() {
    let v = toml_value(&Value::String("line1\nline2 \"q\"".into()));
    // toml string literal: round-trips through the toml parser
    let parsed: toml::Value = format!("k = {v}").parse().unwrap();
    assert_eq!(parsed["k"].as_str().unwrap(), "line1\nline2 \"q\"");
}

#[test]
fn config_overrides_serialize_as_key_value() {
    let mut o = opts();
    o.config.insert(
        "mcp_servers".into(),
        json!({ "github": { "command": "gh-mcp" } }),
    );
    let a = build_args(&o, None, None);
    assert!(a.iter().any(|s| s.starts_with("mcp_servers=")));
}

#[test]
fn resolve_injects_developer_instructions_when_context_on() {
    let req = RunRequest::default();
    let cfg = Config::default();
    let o = resolve(&req, &cfg, None, None, Some("IIICTX"));
    assert_eq!(
        o.config
            .get("developer_instructions")
            .and_then(Value::as_str),
        Some("IIICTX")
    );
}

#[test]
fn resolve_caller_developer_instructions_wins() {
    let req = RunRequest {
        codex_config: Some(json!({ "developer_instructions": "house rules" })),
        ..Default::default()
    };
    let cfg = Config::default();
    let o = resolve(&req, &cfg, None, None, Some("IIICTX"));
    assert_eq!(
        o.config
            .get("developer_instructions")
            .and_then(Value::as_str),
        Some("house rules")
    );
}

#[test]
fn resolve_honors_per_turn_overrides_over_prior() {
    let req = RunRequest {
        cwd: Some("/new".into()),
        model: Some("new-model".into()),
        ..Default::default()
    };
    let cfg = Config::default();
    let o = resolve(&req, &cfg, Some("old-model"), Some("/old"), None);
    assert_eq!(o.cwd, "/new");
    assert_eq!(o.model, "new-model");
}
