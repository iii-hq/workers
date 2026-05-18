use approval_gate::rules::{self, Action, RuleSource};
use approval_gate::WorkerConfig;
use serde_json::json;
use serde_yaml::Value as YamlValue;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn harness_preset_denies_harness_call_before_catch_all_ask() {
    let preset = include_str!("../../harness/approval-gate-rules.yaml");
    let cfg: WorkerConfig = serde_yaml::from_str(preset).expect("harness preset parses");
    let rules = cfg.rules.expect("preset must explicitly enable policy");
    assert_harness_call_denied(&rules);
}

#[test]
fn harness_rules_injector_updates_list_worker_config_and_is_idempotent() {
    let generated = r#"
workers:
  - name: approval-gate
    config:
      approval_state_scope: approvals
      default_timeout_ms: 1234
      topic: agent::before_function_call
  - name: shell
"#;

    let once = run_injector(generated);
    let twice = run_injector(&once);

    assert_eq!(
        once, twice,
        "managed approval rules injection must be idempotent",
    );
    assert_eq!(once.matches("# approval-gate-rules:start").count(), 1);
    assert_eq!(once.matches("# approval-gate-rules:end").count(), 1);

    let cfg = approval_gate_worker_config(&once);
    assert_eq!(cfg.approval_state_scope, "approvals");
    assert_eq!(cfg.default_timeout_ms, 1234);
    assert_eq!(cfg.topic, "agent::before_function_call");
    assert_harness_call_denied(&cfg.rules.expect("injected rules"));
}

#[test]
fn harness_rules_injector_creates_config_block_for_mapping_worker_config() {
    let generated = r#"
workers:
  approval-gate:
    worker_path: /tmp/approval-gate
  shell:
    worker_path: /tmp/shell
"#;

    let injected = run_injector(generated);
    let cfg = approval_gate_worker_config(&injected);

    assert_harness_call_denied(&cfg.rules.expect("injected rules"));
    assert!(
        injected.contains("approval-gate:\n    config:\n"),
        "mapping-style worker without config must receive a config block before worker_path: {injected}",
    );
}

fn assert_harness_call_denied(rules: &rules::Ruleset) {
    let ctx = rules::match_context("harness::call", &json!({"function_id": "run::start"}));
    let matched = rules::evaluate_context(rules, RuleSource::Global, &ctx).expect("match");
    assert_eq!(matched.action, Action::Deny);
    assert_eq!(matched.permission, "harness::call");
    assert_eq!(
        matched.reason.as_deref(),
        Some("harness dispatch cannot bypass approval policy"),
    );
}

fn run_injector(config_text: &str) -> String {
    let repo = repo_root();
    let script = repo.join("harness/scripts/apply-approval-gate-rules.py");
    let rules = repo.join("harness/approval-gate-rules.yaml");
    let config = temp_config_path();

    fs::write(&config, config_text).expect("write temp generated config");
    let output = Command::new("python3")
        .arg(script)
        .arg("--config")
        .arg(&config)
        .arg("--rules")
        .arg(rules)
        .output()
        .expect("run approval-gate rules injector");

    if !output.status.success() {
        panic!(
            "injector failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let injected = fs::read_to_string(&config).expect("read injected config");
    let _ = fs::remove_file(config);
    injected
}

fn approval_gate_worker_config(config_text: &str) -> WorkerConfig {
    let root: YamlValue = serde_yaml::from_str(config_text).expect("generated config parses");
    let workers = root
        .as_mapping()
        .and_then(|m| m.get(&YamlValue::String("workers".into())))
        .expect("workers root");

    let config = if let Some(seq) = workers.as_sequence() {
        seq.iter()
            .find_map(|worker| {
                let map = worker.as_mapping()?;
                let name = map.get(&YamlValue::String("name".into()))?.as_str()?;
                if name != "approval-gate" {
                    return None;
                }
                map.get(&YamlValue::String("config".into())).cloned()
            })
            .expect("approval-gate list worker config")
    } else {
        let worker = workers
            .as_mapping()
            .and_then(|m| m.get(&YamlValue::String("approval-gate".into())))
            .expect("approval-gate mapping worker");
        worker
            .as_mapping()
            .and_then(|m| m.get(&YamlValue::String("config".into())))
            .cloned()
            .expect("approval-gate mapping worker config")
    };

    serde_yaml::from_value(config).expect("approval-gate worker config deserializes")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("approval-gate parent repo")
        .to_path_buf()
}

fn temp_config_path() -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "approval-gate-rules-injector-{}-{nonce}.yaml",
        std::process::id()
    ))
}
