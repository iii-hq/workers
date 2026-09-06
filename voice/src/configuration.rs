//! Integration with the `configuration` worker: register the schema, fetch the
//! authoritative value at boot, and hot-reload it when it changes.
//!
//! `configuration` is a REQUIRED boot dependency: a failed register or fetch
//! aborts startup rather than running on a guessed model or endpoint.

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

pub const CONFIG_ID: &str = "voice";
const CONFIG_FN_ID: &str = "voice::on-config-change";
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries, multiplied by the attempt
/// number for a linear backoff.
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register this worker's configuration schema. When `seed` is present its
/// value becomes `initial_value`; otherwise the built-in default is seeded only
/// when nothing is stored yet, so calling this every boot is safe.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Voice",
        "description": "Speech-to-text engine and model, utterance endpointing, read-aloud \
                        engine, and the size and session limits of the voice worker.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": CONFIG_ID },
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
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

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))
}

/// `Ok(None)` when the entry does not exist. The engine's missing-entry codes
/// vary in case, so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID }))
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
/// authoritative value, so this carries only the advisory id.
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
pub fn register_config_trigger(
    iii: &IIIClient,
    cell: ConfigCell,
    on_reload: Arc<dyn Fn(Arc<WorkerConfig>) + Send + Sync>,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            let on_reload = on_reload.clone();
            async move {
                if let Some(cfg) = on_config_change(&engine, &cell).await {
                    on_reload(cfg);
                }
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the voice worker from the authoritative configuration when \
             it changes, swapping the per-call snapshot.",
        ),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
    ))?;
    Ok(())
}

/// Reload from the AUTHORITATIVE configuration. Returns the new snapshot when
/// one was applied.
///
/// The caller-supplied trigger payload is deliberately ignored:
/// `voice::on-config-change` is a bus function, so trusting a `new_value` in
/// the payload would let any caller repoint the transcription endpoint.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell) -> Option<Arc<WorkerConfig>> {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous config"
            );
            return None;
        }
    };
    apply_config(cell, cfg).await;
    tracing::info!("voice configuration reloaded");
    Some(cell.read().await.clone())
}

/// `true` for the one error that is an answer rather than a failure: the entry
/// does not exist yet.
fn is_not_found(error: &str) -> bool {
    error.to_ascii_uppercase().contains("NOT_FOUND")
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
    use super::*;

    #[test]
    fn a_missing_entry_is_not_retried() {
        assert!(is_not_found(
            "remote error (NOT_FOUND): configuration 'voice' not found"
        ));
        assert!(!is_not_found("connection reset by peer"));
    }

    #[tokio::test]
    async fn apply_config_swaps_the_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        apply_config(
            &cell,
            WorkerConfig {
                max_sessions: 3,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.max_sessions, 3);
    }

    /// The config-change handler must stay off the public catalog.
    #[test]
    fn the_reload_handler_is_not_on_the_public_catalog() {
        let ids: Vec<&str> = crate::functions::catalog()
            .iter()
            .map(|s| s.function_id)
            .collect();
        assert!(!ids.contains(&CONFIG_FN_ID));
    }
}
