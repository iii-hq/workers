//! Integration with the `configuration` worker: register the schema, fetch the
//! authoritative value at boot, and hot-reload it when it changes.
//!
//! Every field here is a per-call tuning knob read from the live snapshot, so
//! there is nothing structural to rebuild and nothing that needs a restart.
//!
//! `configuration` is a REQUIRED boot dependency: a failed register or fetch
//! aborts startup rather than running on a guessed size ceiling.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler. A handler takes a
/// `read().await`, clones the inner `Arc` out, and drops the lock before doing
/// any work; `apply_config` replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "canvas";

/// Process-stable entry identity; the form family remains CONFIG_ID.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONFIG_ID.to_string())
    })
    .as_str()
}
const CONFIG_FN_ID: &str = "canvas::on-config-change";
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries, multiplied by the attempt
/// number for a linear backoff.
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the schema without replacing an existing value. The optional seed
/// or built-in default becomes `initial_value` only on the first registration.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Canvas",
        "description": "Limits for storing diagrams: the size ceiling on one canvas source \
                        (mermaid text or an excalidraw scene) and the cap on how many records \
                        one canvas::list response returns.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": CONFIG_ID },
    });
    if should_seed_default_value(iii).await? {
        payload["initial_value"] = seed.cloned().unwrap_or_default().to_json();
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live configuration (env-expanded by the configuration worker;
/// `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in defaults");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

/// Require a value at the resolved entry ID so reload cannot silently switch to defaults.
async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{}` not found", config_id()))
}

/// `Ok(None)` when the entry does not exist. The engine's missing-entry codes
/// vary in case, so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Payload of the internal config-change handler. The handler re-fetches the
/// authoritative value, so this carries only the advisory id; a struct rather
/// than a `Value` keeps the request schema concrete.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal config-change handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. The handler re-fetches via `configuration::get` and ignores the
/// trigger payload, so a direct call can never inject config.
pub fn register_config_trigger(iii: &IIIClient, cell: ConfigCell) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the canvas worker from the authoritative configuration when \
             it changes, swapping the per-call snapshot.",
        )
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

/// Reload from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is deliberately ignored:
/// `canvas::on-config-change` is a bus function, so trusting a `new_value` in
/// the payload would let any caller lift the size ceiling without touching
/// persisted state.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous config"
            );
            return;
        }
    };
    apply_config(cell, cfg).await;
    tracing::info!("canvas configuration reloaded");
}

/// `true` for the one error that is an answer rather than a failure: the entry
/// does not exist yet. Retrying it wastes the backoff on every first boot and
/// logs two warnings for a completely normal state.
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
                    timeout_ms: None,
                }
                .namespace("default"),
            )
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                if is_not_found(&last_err) {
                    return Err(last_err);
                }
                if attempt < CONFIG_RETRIES {
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_err,
                        "configuration RPC failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * u64::from(attempt),
                    ))
                    .await;
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

    use super::*;

    /// A missing entry is the normal first-boot state, not a transient
    /// failure. Retrying it spends the whole backoff and logs warnings on
    /// every clean install.
    #[test]
    fn a_missing_entry_is_not_retried() {
        assert!(is_not_found(
            "remote error (NOT_FOUND): configuration 'canvas' not found"
        ));
        assert!(!is_not_found("STATEMENT_NOT_FOUND"));
        assert!(!is_not_found("RESOURCE_NOT_FOUND"));
        assert!(!is_not_found("NOT_FOUND_EXTRA"));
        assert!(!is_not_found("remote error (ADAPTER_ERROR): NOT_FOUND"));
        assert!(!is_not_found(
            "remote error (OTHER): remote error (NOT_FOUND): nested"
        ));
        assert!(!is_not_found("connection reset by peer"));
        assert!(!is_not_found("timed out"));
    }

    #[tokio::test]
    async fn apply_config_swaps_the_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.max_list, WorkerConfig::default().max_list);

        apply_config(
            &cell,
            WorkerConfig {
                max_list: 7,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.max_list, 7);
    }

    /// The config-change handler must stay off the public catalog: it is
    /// registered here, not in `functions::register_all`.
    #[test]
    fn the_reload_handler_is_not_on_the_public_catalog() {
        let ids: Vec<&str> = crate::functions::catalog()
            .iter()
            .map(|s| s.function_id)
            .collect();
        assert!(!ids.contains(&CONFIG_FN_ID));
    }
}
