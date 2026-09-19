//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors context-manager.
//!
//! `data_dir` is the one STRUCTURAL field (the boot signature): a change
//! reopens the store and swaps it into the shared cell (last-good on
//! failure). Every other field is read from the live snapshot per call.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::store::Store;

/// Hot-swappable config snapshot shared with every handler.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;
/// Hot-swappable store handle (rebuilt on a `data_dir` change).
pub type StoreCell = Arc<RwLock<Arc<Store>>>;

pub const DEFAULT_CONFIG_ID: &str = "memory";

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
const CONFIG_FN_ID: &str = "memory::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the schema. The optional seed or built-in default is installed
/// only when no stored value exists; live configuration always takes precedence.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Memory",
        "description": "Cross-session agent memory: bank data directory, default bank, \
                        per-turn injection toggles and budgets, and the post-turn \
                        extraction model/window.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // A seed initializes an absent entry; it never replaces a Compose override.
    if should_seed_default_value(iii).await? {
        payload["initial_value"] = seed
            .map(|value| value.to_json())
            .unwrap_or_else(|| WorkerConfig::default().to_json());
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `memory` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

/// `true` for the one error that is a definitive answer rather than a
/// failure: the configuration worker's uppercase `NOT_FOUND` entry code.
/// Matched case-SENSITIVELY — the engine's missing-function code is the
/// lowercase `function_not_found` and a backend lookup failure is
/// `statement_not_found`; those must propagate as errors instead of being
/// read as "nothing stored yet", which would seed a default over a stored
/// operator/override value.
fn is_not_found(error: &str) -> bool {
    error.contains("NOT_FOUND")
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// Returns `Ok(None)` when the entry does not exist. The engine's
/// missing-entry code is the uppercase `NOT_FOUND`; match it case-SENSITIVELY so a
/// service/transport failure (`function_not_found`/`statement_not_found`)
/// propagates as an error instead of seeding over a stored value.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        // A successful reply MUST carry `value`; treating its absence as
        // "no entry" would let a malformed response seed defaults over an
        // intended configuration.
        Ok(resp) => resp
            .get("value")
            .cloned()
            .map(Some)
            .ok_or_else(|| "configuration::get returned no `value` field".to_string()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Internal `memory::on-config-change` trigger payload. The handler
/// re-fetches the authoritative configuration (trusting `new_value` from a
/// discoverable bus function would let any caller inject config).
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger.
pub fn register_config_trigger(
    iii: &IIIClient,
    cell: ConfigCell,
    store: StoreCell,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let store_for_fn = store.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let store = store_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &store)
                    .await
                    .map_err(Error::Handler)?;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload memory from the authoritative configuration when it changes — \
             reopens the store on a data_dir change and swaps the per-call tuning snapshot \
             otherwise.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
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

/// Serializes reloads: concurrent configuration events would otherwise
/// race the store/config swaps and could commit an older fetch over a
/// newer one.
static RELOAD_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Reload from the AUTHORITATIVE configuration. When `data_dir` changed,
/// reopen the store first; a reopen failure keeps the previous store AND
/// config (last-good) so the live snapshot's `data_dir` never diverges
/// from the store actually in use. Store and config commit under one held
/// config write guard, so no handler can observe the new store with the
/// old configuration. Errors propagate so the configuration worker sees
/// the failed update instead of `{ ok: true }`.
async fn on_config_change(
    iii: &IIIClient,
    cell: &ConfigCell,
    store: &StoreCell,
) -> Result<(), String> {
    let _reload = RELOAD_GATE.lock().await;
    let cfg = fetch_config(iii)
        .await
        .map_err(|e| format!("config-change fetch failed (previous config kept): {e}"))?;

    let mut cfg_guard = cell.write().await;
    if cfg_guard.boot_signature() != cfg.boot_signature() {
        let next = Store::open(cfg.resolved_data_dir()).map_err(|e| {
            format!("config-change store reopen failed (previous store and config kept): {e}")
        })?;
        *store.write().await = Arc::new(next);
        tracing::info!("memory store reopened (data_dir changed)");
    }
    *cfg_guard = Arc::new(cfg);
    tracing::info!("memory configuration reloaded");
    Ok(())
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
                    tracing::warn!(function_id, attempt, error = %last_err, "configuration RPC failed; retrying");
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
    #[test]
    fn missing_entry_detection_does_not_mask_service_failures() {
        assert!(super::is_not_found("NOT_FOUND"));
        assert!(super::is_not_found(
            "configuration::get failed after 3 attempts: NOT_FOUND"
        ));
        assert!(!super::is_not_found("function_not_found"));
        assert!(!super::is_not_found("statement_not_found"));
        assert!(!super::is_not_found(
            "configuration::get failed after 3 attempts: function_not_found"
        ));
    }

    use super::*;

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.recall_limit, 6);
        let tuned = WorkerConfig {
            recall_limit: 12,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.recall_limit, 12);
    }
}
