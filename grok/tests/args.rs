//! Arg + config resolution tests for the headless Grok invocation.

use grok::config::Config;
use grok::functions::types::RunRequest;
use grok::grok::args::{build_args, resolve, ResolvedOptions};

fn opts() -> ResolvedOptions {
    ResolvedOptions {
        model: String::new(),
        cwd: String::new(),
        always_approve: false,
        additional_directories: vec![],
        instructions: None,
    }
}

#[test]
fn builds_base_streaming_json_invocation() {
    let a = build_args("hello", &opts(), None);
    assert_eq!(a[0], "--print");
    assert_eq!(a[1], "hello");
    let f = a.iter().position(|s| s == "--output-format").unwrap();
    assert_eq!(a[f + 1], "streaming-json");
    assert!(a.contains(&"--no-alt-screen".to_string()));
    assert!(a.contains(&"--no-auto-update".to_string()));
}

#[test]
fn resume_appends_session_flag() {
    let a = build_args("hi", &opts(), Some("sess-1"));
    let i = a
        .iter()
        .position(|s| s == "--session")
        .expect("--session present");
    assert_eq!(a[i + 1], "sess-1");
}

#[test]
fn always_approve_flag_emitted_when_set() {
    let mut o = opts();
    o.always_approve = true;
    let a = build_args("hi", &o, None);
    assert!(a.contains(&"--always-approve".to_string()));
}

#[test]
fn model_cwd_and_add_dir_mapped() {
    let mut o = opts();
    o.model = "grok-build-0.1".into();
    o.cwd = "/repo".into();
    o.additional_directories = vec!["/extra".into()];
    let a = build_args("hi", &o, None);
    let m = a.iter().position(|s| s == "--model").unwrap();
    assert_eq!(a[m + 1], "grok-build-0.1");
    let c = a.iter().position(|s| s == "--cwd").unwrap();
    assert_eq!(a[c + 1], "/repo");
    let d = a.iter().position(|s| s == "--add-dir").unwrap();
    assert_eq!(a[d + 1], "/extra");
}

#[test]
fn resolve_carries_iii_instructions_when_context_on() {
    let req = RunRequest::default();
    let cfg = Config::default();
    let o = resolve(&req, &cfg, None, None, Some("IIICTX"));
    assert_eq!(o.instructions.as_deref(), Some("IIICTX"));
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

#[test]
fn resolve_falls_back_to_prior_then_config_defaults() {
    let req = RunRequest::default();
    let cfg = Config::default();
    let o = resolve(&req, &cfg, Some("prior-model"), Some("/prior"), None);
    assert_eq!(o.model, "prior-model");
    assert_eq!(o.cwd, "/prior");
}
