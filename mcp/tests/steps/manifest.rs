//! Step defs for `tests/features/manifest.feature`.
//!
//! Subprocess-driven: shells out to the compiled `mcp` binary and
//! asserts against `--manifest` / `--help`. No engine required.

use std::process::Command;

use cucumber::{then, when};
use serde_json::Value;

use crate::common::world::McpWorld;

const MANIFEST_OUTPUT: &str = "manifest_stdout";
const MANIFEST_STATUS: &str = "manifest_status";
const HELP_OUTPUT: &str = "help_stdout";
const HELP_STATUS: &str = "help_status";

fn cargo_bin() -> &'static str {
    // `env!` is a compile-time macro; it pulls in the path Cargo
    // exposes for the named binary built by this crate.
    env!("CARGO_BIN_EXE_mcp")
}

#[when("I run `mcp --manifest`")]
fn run_manifest(world: &mut McpWorld) {
    let out = Command::new(cargo_bin())
        .arg("--manifest")
        .output()
        .expect("spawn mcp --manifest");
    world
        .stash
        .insert(MANIFEST_STATUS.into(), Value::Bool(out.status.success()));
    let stdout = String::from_utf8(out.stdout).expect("manifest stdout is utf-8");
    world
        .stash
        .insert(MANIFEST_OUTPUT.into(), Value::String(stdout));
}

#[when("I run `mcp --help`")]
fn run_help(world: &mut McpWorld) {
    let out = Command::new(cargo_bin())
        .arg("--help")
        .output()
        .expect("spawn mcp --help");
    world
        .stash
        .insert(HELP_STATUS.into(), Value::Bool(out.status.success()));
    let stdout = String::from_utf8(out.stdout).expect("help stdout is utf-8");
    world
        .stash
        .insert(HELP_OUTPUT.into(), Value::String(stdout));
}

fn manifest_value(world: &McpWorld) -> Value {
    let stdout = world
        .stash
        .get(MANIFEST_OUTPUT)
        .and_then(|v| v.as_str())
        .expect("--manifest stdout missing; run the When step first");
    serde_json::from_str(stdout).expect("--manifest stdout is valid JSON")
}

#[then("the exit status is success")]
fn exit_status_success(world: &mut McpWorld) {
    let ok = world
        .stash
        .get(MANIFEST_STATUS)
        .or_else(|| world.stash.get(HELP_STATUS))
        .and_then(|v| v.as_bool())
        .expect("a When step should have stored an exit status");
    assert!(ok, "binary exited non-zero");
}

#[then("stdout is valid JSON")]
fn stdout_valid_json(world: &mut McpWorld) {
    let _ = manifest_value(world);
}

#[then(regex = r#"^the manifest field "(\w+)" is "(.+)"$"#)]
fn manifest_field_eq(world: &mut McpWorld, key: String, want: String) {
    let m = manifest_value(world);
    let got = m
        .get(&key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("manifest.{key} missing or not a string"));
    assert_eq!(got, want);
}

#[then("the manifest field \"version\" matches the crate version")]
fn manifest_version_matches(world: &mut McpWorld) {
    let m = manifest_value(world);
    assert_eq!(
        m["version"].as_str().expect("version is a string"),
        env!("CARGO_PKG_VERSION")
    );
}

#[then(regex = r#"^the manifest's default api_path is "([^"]+)"$"#)]
fn manifest_api_path(world: &mut McpWorld, want: String) {
    let m = manifest_value(world);
    let got = m["default_config"]["api_path"]
        .as_str()
        .expect("default_config.api_path missing");
    assert_eq!(got, want);
}

#[then(regex = r#"^the manifest's default require_expose is (true|false)$"#)]
fn manifest_require_expose(world: &mut McpWorld, want: String) {
    let m = manifest_value(world);
    let got = m["default_config"]["require_expose"]
        .as_bool()
        .expect("default_config.require_expose is a bool");
    assert_eq!(got, want == "true");
}

#[then(regex = r#"^the manifest's default hidden_prefixes contains "([^"]+)"$"#)]
fn manifest_hidden_prefix(world: &mut McpWorld, needle: String) {
    let m = manifest_value(world);
    let arr = m["default_config"]["hidden_prefixes"]
        .as_array()
        .expect("default_config.hidden_prefixes is an array");
    let prefixes: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        prefixes.contains(&needle.as_str()),
        "hidden_prefixes missing {needle:?}; got {prefixes:?}"
    );
}

#[then("the manifest's supported_targets is non-empty")]
fn manifest_supported_targets(world: &mut McpWorld) {
    let m = manifest_value(world);
    let arr = m["supported_targets"]
        .as_array()
        .expect("supported_targets is an array");
    assert!(!arr.is_empty(), "supported_targets is empty");
}

#[then(regex = r#"^stdout mentions "([^"]+)"$"#)]
fn stdout_mentions(world: &mut McpWorld, needle: String) {
    let stdout = world
        .stash
        .get(HELP_OUTPUT)
        .or_else(|| world.stash.get(MANIFEST_OUTPUT))
        .and_then(|v| v.as_str())
        .expect("a When step should have stored stdout");
    assert!(
        stdout.contains(&needle),
        "stdout does not mention {needle:?}; got: {stdout}"
    );
}
