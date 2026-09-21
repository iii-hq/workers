//! Capability-based initialization for modern and legacy configuration services.
//!
//! Also compiled by workers with their own SDK pins via `#[path]`; this module
//! depends only on serde_json, tracing and std, not on UI plumbing or an SDK.
use std::collections::HashSet;
use std::future::Future;
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

/// Prefer atomic ensure. Only its exact missing-function error enables the
/// non-atomic legacy get/register path. The caller owns routing and retries.
pub async fn ensure_with<F, Fut>(mut payload: Value, mut call: F) -> Result<(), String>
where
    F: FnMut(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    match call("configuration::ensure", payload.clone()).await {
        Ok(_) => return Ok(()),
        Err(error) if remote_code(&error, "configuration::ensure", "function_not_found") => {}
        Err(error) => return Err(error),
    }
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .ok_or("configuration initialization requires an id")?
        .to_owned();
    warn_legacy_once(&id);
    let existing = match call("configuration::get", json!({ "id": id, "raw": true })).await {
        Ok(response) => response
            .as_object()
            .and_then(|object| object.get("value"))
            .cloned()
            .ok_or("configuration::get returned no `value` field")?,
        Err(error) if remote_code(&error, "configuration::get", "NOT_FOUND") => Value::Null,
        Err(error) => return Err(error),
    };
    if !existing.is_null() {
        // Omit, never copy the stored value: copying would overwrite a later edit
        // and could persist expanded templates. Even false/0/"" are stored values.
        if let Some(object) = payload.as_object_mut() {
            object.remove("initial_value");
        }
    }
    // In a legacy engine another writer can still race this read/register pair.
    // No client-local lock or extra read can supply the engine's atomic guarantee.
    call("configuration::register", payload).await.map(|_| ())
}

/// Match the SDK Display envelope, optionally wrapped by the existing three-attempt
/// retry ladder. Never search message text, accept another wrapper, or fold case.
fn remote_code(error: &str, function: &str, code: &str) -> bool {
    let raw = error.trim();
    let wrapper = format!("{function} failed after 3 attempts: ");
    let raw = raw.strip_prefix(&wrapper).unwrap_or(raw);
    let envelope = format!("remote error ({code}):");
    raw == code || raw == envelope || raw.starts_with(&format!("{envelope} "))
}

fn warn_legacy_once(id: &str) {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let first = WARNED
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id.to_owned());
    if first {
        tracing::warn!(id, "engine lacks configuration::ensure; using non-atomic legacy initialization; upgrade to >=0.24.1 for concurrent-write safety");
    }
}
