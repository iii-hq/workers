//! Public-contract tests. CI's validate_worker.py requires a non-empty
//! tests/ suite for source-changed workers; the deep per-builder coverage
//! lives in the unit tests under src/functions/, and the process mechanics
//! in src/gh.rs.

use github::config::Config;
use github::functions::{catalog, passthrough, pr};
use serde_json::json;

/// The engine injects `_caller_worker_id` into every payload; typed requests
/// must tolerate it (serde's default unknown-field behavior — pinned here so
/// nobody adds `deny_unknown_fields` and breaks every live call).
#[test]
fn requests_tolerate_engine_injected_metadata() {
    let r: pr::ListRequest = serde_json::from_value(json!({
        "repo": "o/r",
        "_caller_worker_id": "harness",
    }))
    .expect("live shape parses");
    assert_eq!(pr::list_args(&r)[..4], ["pr", "list", "-R", "o/r"]);

    let r: passthrough::ExecRequest = serde_json::from_value(json!({
        "args": ["--version"],
        "_caller_worker_id": "harness",
    }))
    .expect("live shape parses");
    assert_eq!(r.args, vec!["--version"]);
}

#[test]
fn config_defaults_are_the_documented_ones() {
    let c = Config::default();
    assert_eq!(c.gh_bin(), "gh");
    assert!(c.token.is_empty());
    assert_eq!(c.default_timeout_ms, 30_000);
    assert_eq!(c.max_timeout_ms, 120_000);
    assert_eq!(c.max_output_bytes, 1_048_576);
}

#[test]
fn resolve_timeout_clamps_to_the_configured_max() {
    let c = Config::default();
    assert_eq!(c.resolve_timeout(None), 30_000);
    assert_eq!(c.resolve_timeout(Some(5)), 5);
    assert_eq!(c.resolve_timeout(Some(999_999)), 120_000);
}

/// `${NAME}` in the seed expands from the process env; untouched fields keep
/// their defaults; a missing file yields the full defaults.
#[test]
fn seed_loading_expands_env_and_defaults_the_rest() {
    // Unique var name: no other test reads or writes it, so the process-env
    // mutation cannot race the parallel test runner.
    std::env::set_var("GH_CONTRACT_TOKEN_92AF", "sekrit");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "token: \"${GH_CONTRACT_TOKEN_92AF}\"\ndefault_timeout_ms: 1000\n",
    )
    .unwrap();
    let c = Config::load(path.to_str().unwrap()).unwrap();
    std::env::remove_var("GH_CONTRACT_TOKEN_92AF");
    assert_eq!(c.token, "sekrit");
    assert_eq!(c.default_timeout_ms, 1000);
    assert_eq!(c.max_timeout_ms, 120_000);

    let c = Config::load(dir.path().join("nope.yaml").to_str().unwrap()).unwrap();
    assert_eq!(c.default_timeout_ms, 30_000);
}

#[test]
fn gh_bin_prefers_the_configured_path() {
    let c = Config {
        gh_executable: "/opt/homebrew/bin/gh".to_string(),
        ..Config::default()
    };
    assert_eq!(c.gh_bin(), "/opt/homebrew/bin/gh");
}

/// 30 curated functions + exec + api + 4 webhook monitoring operations. The exact ids and order are pinned in
/// tests/schemas.rs; this is the cheap headcount.
#[test]
fn catalog_covers_the_full_surface() {
    assert_eq!(catalog().len(), 37);
}

/// The registry publishes `iii.worker.yaml`'s `config:` block, not config.yaml.
/// Keep them identical (minus the environment-specific token) so a registry
/// install gets the same defaults as a source checkout.
#[test]
fn manifest_config_matches_config_yaml() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(root.join("iii.worker.yaml")).unwrap())
            .unwrap();
    let mut seed: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(root.join("config.yaml")).unwrap()).unwrap();
    seed.as_mapping_mut().unwrap().remove("token");
    assert_eq!(
        manifest["config"], seed,
        "iii.worker.yaml config: must mirror config.yaml (except token)"
    );
    // Both parse as a valid worker configuration.
    let parsed: github::config::Config =
        serde_json::from_value(serde_json::to_value(&manifest["config"]).unwrap()).unwrap();
    assert_eq!(
        parsed.webhooks.notifications.profile,
        github::webhooks::notifications::NotificationProfile::AgentActionable
    );
    // quick-tunnel is installed on demand, never as a hard dependency.
    assert!(manifest["dependencies"].get("quick-tunnel").is_none());
}
