//! Shared step-def utilities: trigger the coder surface, write fixtures
//! into the per-scenario `base_path`, and pull stashed outcomes back
//! out for `Then` assertions.

use cucumber::gherkin::Step;
use iii_sdk::TriggerRequest;
use serde_json::Value;

use crate::common::world::CoderWorld;

pub const LAST_OK: &str = "coder_last_ok";
pub const LAST_ERR: &str = "coder_last_err";

/// Strip Cucumber's leading docstring newline. Cucumber inserts a `\n`
/// right after the opening `"""`; tests that care about exact file
/// contents (e.g. fixture reads) need that newline gone.
pub fn docstring_body(step: &Step) -> String {
    let raw = step.docstring.as_deref().unwrap_or("");
    raw.strip_prefix('\n').unwrap_or(raw).to_string()
}

/// Parse a docstring as a JSON payload. Panics with the offending text
/// when invalid so failures point at the feature line.
pub fn parse_payload(step: &Step) -> Value {
    let raw = step.docstring.as_deref().unwrap_or("{}");
    serde_json::from_str(raw)
        .unwrap_or_else(|e| panic!("payload docstring is not valid JSON: {raw:?} \u{2014} {e}"))
}

/// Trigger a coder function over the shared SDK handle, then stash
/// either the success `Value` (`LAST_OK`) or the error string
/// (`LAST_ERR`). No-op when no engine is reachable (soft skip).
pub async fn call_coder(world: &mut CoderWorld, function_id: &str, payload: Value) {
    world.stash.remove(LAST_OK);
    world.stash.remove(LAST_ERR);
    let Some(iii) = world.iii.clone() else {
        return;
    };
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
    {
        Ok(v) => {
            world.stash.insert(LAST_OK.into(), v);
        }
        Err(e) => {
            world
                .stash
                .insert(LAST_ERR.into(), Value::String(e.to_string()));
        }
    }
}

/// Write a fixture file under the per-scenario `base_path`. Creates
/// parent directories as needed. No-op when no engine is reachable.
pub fn write_fixture(world: &CoderWorld, rel: &str, body: &str) {
    let Some(base) = world.base_path.as_ref() else {
        return;
    };
    let path = base.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create_dir_all");
    }
    std::fs::write(&path, body).expect("write fixture");
}

/// Create a directory under the per-scenario `base_path`. No-op when
/// no engine is reachable.
pub fn mkdir(world: &CoderWorld, rel: &str) {
    let Some(base) = world.base_path.as_ref() else {
        return;
    };
    let path = base.join(rel);
    std::fs::create_dir_all(&path).expect("create_dir_all");
}

/// Stashed success result; panics if missing so the failure message
/// surfaces the last error (rather than a vague "missing key").
pub fn last_ok(world: &CoderWorld) -> &Value {
    world.stash.get(LAST_OK).unwrap_or_else(|| {
        panic!(
            "no successful call recorded; last error: {:?}",
            world.stash.get(LAST_ERR)
        )
    })
}

/// Stashed error string; empty when no error was recorded.
pub fn last_err(world: &CoderWorld) -> String {
    world
        .stash
        .get(LAST_ERR)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Locate a batch result entry by `path` in `results[]`. Returns `None`
/// when missing. Shared by create-file / update-file / delete-file
/// assertions.
///
/// Result `path`s are canonical-absolute when the input resolved inside
/// the jail and verbatim otherwise; features speak base-relative, so
/// accept either the verbatim form or the path anchored at the
/// scenario's (canonical) base.
pub fn batch_result<'a>(world: &'a CoderWorld, path: &str) -> Option<&'a Value> {
    let abs = world.base_path.as_ref().map(|b| b.join(path));
    let v = world.stash.get(LAST_OK)?;
    let arr = v.get("results")?.as_array()?;
    arr.iter().find(|e| {
        let Some(got) = e["path"].as_str() else {
            return false;
        };
        // Component-wise Path comparison so "." anchors to the base
        // itself rather than the literal "<base>/." string.
        got == path
            || abs
                .as_deref()
                .is_some_and(|a| std::path::Path::new(got) == a)
    })
}
