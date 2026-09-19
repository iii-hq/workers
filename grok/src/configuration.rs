//! Integration with the `configuration` worker — register the grok config
//! schema, fetch the live value, and hot-reload it on change. Every field is a
//! runtime tuning knob (per-turn defaults, stream names, executable path,
//! iii-context toggle), so a change hot-swaps the whole snapshot — there is no
//! security topology to refuse like the path-jail workers.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::Config;

/// Hot-swappable config snapshot shared with every handler. A handler takes a
/// `read().await` and clones the inner `Arc` out (a cheap refcount bump) so it
/// never holds the lock across a turn; `apply_config` whole-snapshot replaces
/// the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<Config>>>;

pub const DEFAULT_CONFIG_ID: &str = "grok";

/// The configuration entry this worker owns.
///
/// `III_CONFIG_NAME` when a supervisor set it, else the built-in name. A worker
/// that hardcodes its id turns that id into a global scarce name: two instances
/// share one entry and take turns overwriting it, and each write wakes both.
/// Being told which entry is its own is what lets them differ.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}
const CONFIG_FN_ID: &str = "grok::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register the schema. The optional seed or built-in default is installed
/// only when no stored value exists; live configuration always takes precedence.
pub async fn register_config(iii: &IIIClient, seed: Option<&Config>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Grok",
        "description": "xAI Grok CLI worker: per-turn defaults (model, working directory, auto-approve), the agent::events / grok::events stream names, the grok CLI path, and whether to inject the iii runtime context into the prompt.",
        "schema": Config::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // A seed initializes an absent entry; it never replaces a Compose override.
    payload["initial_value"] = seed
        .map(|value| value.to_json())
        .unwrap_or_else(|| Config::default().to_json());
    ensure_configuration(iii, payload).await
}

/// Read the live `grok` configuration; built-in default when none stored.
pub async fn fetch_config(iii: &IIIClient) -> Result<Config, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => Config::from_json(&v).map_err(|e| e.to_string()),
        _ => {
            tracing::info!("no grok configuration value found; using built-in defaults");
            Ok(Config::default())
        }
    }
}

/// Forward `payload` to the engine's atomic `configuration::ensure`. Fails
/// CLOSED against an engine that predates it (`function_not_found`): surface
/// the upgrade-required error instead of falling back to the legacy
/// read-then-`register` seed, which could clobber a stored override.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    match trigger_configuration_with_retry(iii, "configuration::ensure", payload).await {
        Ok(_) => Ok(()),
        Err(e) if is_function_not_found(&e) => Err(ENSURE_UNAVAILABLE.to_string()),
        Err(e) => Err(e),
    }
}

/// Upgrade-required error surfaced when the engine lacks atomic
/// `configuration::ensure` (fail CLOSED; never a legacy seed-over-stored write).
const ENSURE_UNAVAILABLE: &str = "configuration::ensure unavailable; upgrade engine with atomic configuration initialization support";

/// `true` when the error is the engine's lowercase missing-FUNCTION envelope
/// `function_not_found` (an engine without `configuration::ensure`). Same
/// envelope discipline as `is_not_found`: peel the one retry wrapper, then
/// require the envelope at the very start so a stray token still propagates.
fn is_function_not_found(error: &str) -> bool {
    const RETRY_WRAPPER: &str = "configuration::ensure failed after 3 attempts: ";
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "function_not_found"
        || raw == "remote error (function_not_found):"
        || raw.starts_with("remote error (function_not_found): ")
}

/// `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(cell: &ConfigCell, cfg: Config) {
    *cell.write().await = Arc::new(cfg);
}

/// Register the internal config-change handler and bind the `configuration`
/// trigger that wakes it.
pub fn register_config_trigger(iii: &IIIClient, cell: ConfigCell) -> Result<(), Error> {
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: Value| {
            let cell = cell.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell).await;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        })
        .request_format(json!({ "type": "object", "properties": {} }))
        .response_format(json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
        }))
        .description("Internal: reload grok configuration when it changes.")
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
    ))?;
    Ok(())
}

/// Re-fetch the authoritative value after trigger registration to close the
/// boot race (an update that landed between the initial fetch and the trigger
/// binding has no other listener).
pub async fn reconcile(iii: &IIIClient, cell: &ConfigCell) {
    on_config_change(iii, cell).await;
}

/// The trigger payload is intentionally ignored — `grok::on-config-change` is
/// a discoverable bus function, so trusting `payload.new_value` would let any
/// caller inject config without updating persisted state. Re-fetch the stored
/// value instead. The previous snapshot is kept on any failure.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(cell, cfg).await;
            tracing::info!("grok configuration reloaded");
        }
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
        }
    }
}

/// `true` only when the error carries the configuration worker's standalone
/// `NOT_FOUND` entry code, identified by the outermost `remote error (<code>)` envelope code rather than a substring or token scan of the message, so
/// a compound code such as `RESOURCE_NOT_FOUND`/`STATEMENT_NOT_FOUND` or the
/// engine's lowercase missing-FUNCTION code `function_not_found` still
/// propagates as a failure instead of being read as "nothing stored yet".
fn is_not_found(error: &str) -> bool {
    // Anchor on the SDK's own rendering instead of scanning the whole string:
    // a remote failure prints as `remote error ({code}): {message}`, and this
    // worker wraps a retried get as
    // `configuration::get failed after CONFIG_RETRIES attempts: {err}`. Peel
    // exactly that one wrapper (never a foreign one or a different attempt
    // count) and then require the NOT_FOUND envelope at the very start, so a
    // NOT_FOUND code buried in an unrelated message, a nested envelope, or a
    // different wrapper stays a real failure and propagates.
    const RETRY_WRAPPER: &str = "configuration::get failed after 3 attempts: ";
    const _: () = assert!(CONFIG_RETRIES == 3);
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "NOT_FOUND"
        || raw == "remote error (NOT_FOUND):"
        || raw.starts_with("remote error (NOT_FOUND): ")
}

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: Some(CONFIG_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                if attempt < CONFIG_RETRIES {
                    // Exponential backoff: 250ms, 500ms, 1000ms, …
                    let backoff = 250u64 << (attempt - 1);
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}

#[cfg(test)]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/config-client/tests/support/is_not_found_cases.rs"
    ));

    /// The missing-entry classifier only seeds on the configuration worker's
    /// standalone `NOT_FOUND` envelope; every unrelated failure or compound
    /// code propagates instead of clobbering a stored value with a default.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(super::is_not_found);
    }
}
