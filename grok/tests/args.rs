//! Arg + config resolution tests for the headless Grok invocation.
//! Flags captured from Grok CLI 0.2.77.

use grok::config::Config;
use grok::functions::types::RunRequest;
use grok::grok::args::{build_args, resolve, ResolvedOptions};

fn opts() -> ResolvedOptions {
    ResolvedOptions {
        model: String::new(),
        cwd: String::new(),
        always_approve: false,
        instructions: None,
    }
}

#[test]
fn builds_base_streaming_json_invocation() {
    let a = build_args("hello", &opts(), None);
    assert_eq!(a[0], "--single");
    assert_eq!(a[1], "hello");
    let f = a.iter().position(|s| s == "--output-format").unwrap();
    assert_eq!(a[f + 1], "streaming-json");
    assert!(a.contains(&"--no-alt-screen".to_string()));
}

#[test]
fn resume_appends_resume_flag() {
    let a = build_args("hi", &opts(), Some("sess-1"));
    let i = a
        .iter()
        .position(|s| s == "--resume")
        .expect("--resume present");
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
fn model_and_cwd_mapped() {
    let mut o = opts();
    o.model = "grok-4.20-0309-non-reasoning".into();
    o.cwd = "/repo".into();
    let a = build_args("hi", &o, None);
    let m = a.iter().position(|s| s == "--model").unwrap();
    assert_eq!(a[m + 1], "grok-4.20-0309-non-reasoning");
    let c = a.iter().position(|s| s == "--cwd").unwrap();
    assert_eq!(a[c + 1], "/repo");
}

#[test]
fn instructions_prepended_on_fresh_turn_only() {
    let mut o = opts();
    o.instructions = Some("IIICTX".into());
    let fresh = build_args("do it", &o, None);
    assert_eq!(fresh[1], "IIICTX\n\ndo it");
    // on resume the session already carries the context
    let resumed = build_args("do it", &o, Some("sess-1"));
    assert_eq!(resumed[1], "do it");
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

#[test]
fn resolve_falls_through_to_config_defaults_when_no_prior() {
    // No payload, no prior session: resolution lands on Config::default().
    let req = RunRequest::default();
    let cfg = Config::default();
    let o = resolve(&req, &cfg, None, None, None);
    assert_eq!(o.model, cfg.defaults.model);
    assert_eq!(o.cwd, cfg.defaults.cwd);
    assert_eq!(o.always_approve, cfg.defaults.always_approve);
}
