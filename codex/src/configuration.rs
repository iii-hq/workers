//! Integration with the `configuration` worker — register the codex config
//! schema, fetch the live value, and hot-reload it on change. Every field is a
//! runtime tuning knob (model/sandbox defaults, stream names, executable path,
//! iii-context toggle), so a change hot-swaps the whole snapshot — there is no
//! security topology to refuse like the path-jail workers.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::Config;

/// Hot-swappable config snapshot shared with every handler. A handler takes a
/// `read().await` and clones the inner `Arc` out (a cheap refcount bump) so it
/// never holds the lock across a turn; `apply_config` whole-snapshot replaces
/// the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<Config>>>;

pub const CONFIG_ID: &str = "codex";
const CONFIG_FN_ID: &str = "codex::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register the `codex` configuration schema. When `seed` is present its value
/// is installed as `initial_value`; otherwise the built-in default is seeded
/// only when no stored value exists yet.
pub async fn register_config(iii: &III, seed: Option<&Config>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Codex",
        "description": "OpenAI Codex worker: per-turn defaults (model, sandbox mode, approval policy, reasoning effort, working directory), the agent::events / codex::events stream names, the codex CLI path, an optional API base URL, and whether to inject the iii runtime context as developer_instructions.",
        "schema": Config::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default(iii).await? {
        payload["initial_value"] = Config::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `codex` configuration; built-in default when none stored.
pub async fn fetch_config(iii: &III) -> Result<Config, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => Config::from_json(&v).map_err(|e| e.to_string()),
        _ => {
            tracing::info!("no codex configuration value found; using built-in defaults");
            Ok(Config::default())
        }
    }
}

async fn should_seed_default(iii: &III) -> Result<bool, String> {
    Ok(matches!(
        try_get_value(iii).await?,
        None | Some(Value::Null)
    ))
}

/// `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_value(iii: &III) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(cell: &ConfigCell, cfg: Config) {
    *cell.write().await = Arc::new(cfg);
}

/// Register the internal config-change handler and bind the `configuration`
/// trigger that wakes it.
pub fn register_config_trigger(iii: &III, cell: ConfigCell) -> Result<(), IIIError> {
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: Value| {
            let cell = cell.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell).await;
                Ok::<Value, IIIError>(json!({ "ok": true }))
            }
        })
        .request_format(json!({ "type": "object", "properties": {} }))
        .response_format(json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
        }))
        .description("Internal: reload codex configuration when it changes."),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
    })?;
    Ok(())
}

/// Re-fetch the authoritative value after trigger registration to close the
/// boot race (an update that landed between the initial fetch and the trigger
/// binding has no other listener).
pub async fn reconcile(iii: &III, cell: &ConfigCell) {
    on_config_change(iii, cell).await;
}

/// The trigger payload is intentionally ignored — `codex::on-config-change` is
/// a discoverable bus function, so trusting `payload.new_value` would let any
/// caller inject config without updating persisted state. Re-fetch the stored
/// value instead. The previous snapshot is kept on any failure.
async fn on_config_change(iii: &III, cell: &ConfigCell) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(cell, cfg).await;
            tracing::info!("codex configuration reloaded");
        }
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
        }
    }
}

async fn trigger_with_retry(iii: &III, function_id: &str, payload: Value) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(CONFIG_TIMEOUT_MS),
            })
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}
