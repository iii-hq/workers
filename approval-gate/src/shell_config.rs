//! Best-effort persistence for `always`-scoped folder-access grants: append
//! the granted directory to the `shell` deployment configuration's
//! `fs.host_roots` list (spec-pr3-approval-gate.md § Resolve orchestration).
//!
//! This is deliberately decoupled from the `shell` worker's own config
//! types — approval-gate only ever touches the single `fs.host_roots`
//! array inside the raw JSON value, so it never needs (or risks drifting
//! from) `shell`'s schema.

use std::sync::OnceLock;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

const SHELL_CONFIG_ID: &str = "shell";

/// Serializes `add_host_root`'s get-modify-set cycle in-process: the
/// `configuration` worker exposes no CAS/version guard, so two concurrent
/// `always`-scope grants for different directories could otherwise both
/// read the same `fs.host_roots` snapshot and the later `configuration::set`
/// would silently clobber the earlier caller's directory.
fn host_root_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

/// Pure: return a NEW configuration value with `dir` appended to
/// `fs.host_roots` (idempotent — a `dir` already present is left alone).
/// Tolerant of a missing/non-object `value`, `fs`, or `host_roots` — each
/// missing piece is synthesized as empty rather than erroring, since the
/// whole call site is best-effort.
pub fn with_host_root(value: &Value, dir: &str) -> Value {
    let mut root = value.as_object().cloned().unwrap_or_default();
    let mut fs = root
        .get("fs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut roots: Vec<Value> = fs
        .get("host_roots")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !roots.iter().any(|v| v.as_str() == Some(dir)) {
        roots.push(Value::String(dir.to_string()));
    }
    fs.insert("host_roots".to_string(), Value::Array(roots));
    root.insert("fs".to_string(), Value::Object(fs));
    Value::Object(root)
}

/// `configuration::get {id:"shell"}` -> `with_host_root` -> `configuration::set`.
/// Every failure (missing `shell` config entry, transport error) is the
/// caller's to log; this never blocks the resolve-execute it precedes.
pub async fn add_host_root(iii: &IIIClient, dir: &str) -> Result<(), Error> {
    let _guard = host_root_lock().lock().await;
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "configuration::get".into(),
            payload: json!({ "id": SHELL_CONFIG_ID }),
            action: None,
            timeout_ms: None,
        })
        .await?;
    let current = reply.get("value").cloned().unwrap_or(Value::Null);
    let next = with_host_root(&current, dir);
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".into(),
        payload: json!({ "id": SHELL_CONFIG_ID, "value": next }),
        action: None,
        timeout_ms: None,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_to_an_empty_value() {
        let next = with_host_root(&Value::Null, "/a/b");
        assert_eq!(next, json!({ "fs": { "host_roots": ["/a/b"] } }));
    }

    #[test]
    fn appends_to_existing_host_roots_without_duplicating() {
        let current = json!({ "fs": { "host_roots": ["/x"] } });
        let once = with_host_root(&current, "/a/b");
        assert_eq!(once["fs"]["host_roots"], json!(["/x", "/a/b"]));
        let twice = with_host_root(&once, "/a/b");
        assert_eq!(twice, once);
    }

    #[test]
    fn preserves_sibling_fields() {
        let current = json!({ "fs": { "host_roots": [], "sandbox": true }, "other": 1 });
        let next = with_host_root(&current, "/a/b");
        assert_eq!(next["other"], json!(1));
        assert_eq!(next["fs"]["sandbox"], json!(true));
        assert_eq!(next["fs"]["host_roots"], json!(["/a/b"]));
    }

    #[test]
    fn tolerates_non_object_value() {
        let next = with_host_root(&json!("garbage"), "/a/b");
        assert_eq!(next, json!({ "fs": { "host_roots": ["/a/b"] } }));
    }

    #[test]
    fn never_mutates_input() {
        let current = json!({ "fs": { "host_roots": ["/x"] } });
        let snapshot = current.clone();
        let _ = with_host_root(&current, "/a/b");
        assert_eq!(current, snapshot);
    }
}
