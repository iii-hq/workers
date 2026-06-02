//! Step defs for `tests/features/manifest.feature`.
//!
//! Subprocess-driven: shells out to the compiled `email` binary and
//! asserts against `--manifest` / `--help`. No engine required.

use std::process::Command;

use cucumber::{then, when};
use serde_json::Value;

use crate::common::world::EmailWorld;

const MANIFEST_OUTPUT: &str = "manifest_stdout";
const MANIFEST_STATUS: &str = "manifest_status";
const HELP_OUTPUT: &str = "help_stdout";
const HELP_STATUS: &str = "help_status";

fn cargo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_email")
}

#[when("I run `email --manifest`")]
fn run_manifest(world: &mut EmailWorld) {
    let out = Command::new(cargo_bin())
        .arg("--manifest")
        .output()
        .expect("spawn email --manifest");
    world
        .stash
        .insert(MANIFEST_STATUS.into(), Value::Bool(out.status.success()));
    let stdout = String::from_utf8(out.stdout).expect("manifest stdout is utf-8");
    world
        .stash
        .insert(MANIFEST_OUTPUT.into(), Value::String(stdout));
}

#[when("I run `email --help`")]
fn run_help(world: &mut EmailWorld) {
    let out = Command::new(cargo_bin())
        .arg("--help")
        .output()
        .expect("spawn email --help");
    world
        .stash
        .insert(HELP_STATUS.into(), Value::Bool(out.status.success()));
    let stdout = String::from_utf8(out.stdout).expect("help stdout is utf-8");
    world
        .stash
        .insert(HELP_OUTPUT.into(), Value::String(stdout));
}

fn manifest_value(world: &EmailWorld) -> Value {
    let stdout = world
        .stash
        .get(MANIFEST_OUTPUT)
        .and_then(|v| v.as_str())
        .expect("--manifest stdout missing; run the When step first");
    serde_json::from_str(stdout).expect("--manifest stdout is valid JSON")
}

#[then("the exit status is success")]
fn exit_status_success(world: &mut EmailWorld) {
    let ok = world
        .stash
        .get(MANIFEST_STATUS)
        .or_else(|| world.stash.get(HELP_STATUS))
        .and_then(|v| v.as_bool())
        .expect("a When step should have stored an exit status");
    assert!(ok, "binary exited non-zero");
}

#[then("stdout is valid JSON")]
fn stdout_valid_json(world: &mut EmailWorld) {
    let _ = manifest_value(world);
}

#[then(regex = r#"^the manifest field "(\w+)" is "(.+)"$"#)]
fn manifest_field_eq(world: &mut EmailWorld, key: String, want: String) {
    let m = manifest_value(world);
    let got = m
        .get(&key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("manifest.{key} missing or not a string"));
    assert_eq!(got, want);
}

#[then("the manifest field \"version\" matches the crate version")]
fn manifest_version_matches(world: &mut EmailWorld) {
    let m = manifest_value(world);
    assert_eq!(
        m["version"].as_str().expect("version is a string"),
        env!("CARGO_PKG_VERSION")
    );
}

#[then(regex = r#"^the manifest lists the function "([^"]+)"$"#)]
fn manifest_lists_function(world: &mut EmailWorld, want: String) {
    let m = manifest_value(world);
    let arr = m["functions"].as_array().expect("functions is an array");
    let ids: Vec<&str> = arr.iter().filter_map(|v| v["id"].as_str()).collect();
    assert!(
        ids.contains(&want.as_str()),
        "function {want:?} missing; got {ids:?}"
    );
}

#[then("the manifest lists exactly 8 functions")]
fn manifest_function_count(world: &mut EmailWorld) {
    let m = manifest_value(world);
    let arr = m["functions"].as_array().expect("functions is an array");
    assert_eq!(arr.len(), 8, "expected 8 functions, got {}", arr.len());
}

#[then(regex = r#"^the manifest lists the trigger type "([^"]+)"$"#)]
fn manifest_lists_trigger_type(world: &mut EmailWorld, want: String) {
    let m = manifest_value(world);
    let arr = m["trigger_types"]
        .as_array()
        .expect("trigger_types is an array");
    let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(
        names.contains(&want.as_str()),
        "trigger type {want:?} missing; got {names:?}"
    );
}

#[then(regex = r#"^the manifest default limits\.([a-z_]+) is (\d+)$"#)]
fn manifest_default_limit(world: &mut EmailWorld, key: String, want: u64) {
    let m = manifest_value(world);
    let got = m["default_config"]["limits"][&key]
        .as_u64()
        .unwrap_or_else(|| panic!("default_config.limits.{key} missing or not an integer"));
    assert_eq!(got, want);
}

#[then(regex = r#"^stdout mentions "([^"]+)"$"#)]
fn stdout_mentions(world: &mut EmailWorld, needle: String) {
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
