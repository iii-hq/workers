//! Integration with the `configuration` worker — register the provider-xai
//! worker-config schema (Agent Tools toggle), fetch the live value, and
//! hot-reload it on change so an operator can flip tools on/off in the console
//! configuration sidebar without a restart. Unlike credentials (router-owned),
//! this is provider behaviour, so it lives in the worker's own config entry.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with the stream handler.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "provider-xai";
const CONFIG_FN_ID: &str = "provider::xai::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

pub fn new_cell() -> ConfigCell {
    Arc::new(RwLock::new(Arc::new(WorkerConfig::default())))
}

/// Register the `provider-xai` configuration schema, seeding the default only
/// when no stored value exists yet.
pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "xAI Provider",
        "description": "xAI provider behaviour: enable Agent Tools (live X / web search via the /v1/responses API) and pick which server-side tools to offer. Credentials and endpoints are managed by the llm-router configuration entry, not here.",
        "schema": WorkerConfig::json_schema(),
    });
    if should_seed_default(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => WorkerConfig::from_json(&v).map_err(|e| e.to_string()),
        _ => Ok(WorkerConfig::default()),
    }
}

async fn should_seed_default(iii: &IIIClient) -> Result<bool, String> {
    Ok(matches!(
        try_get_value(iii).await?,
        None | Some(Value::Null)
    ))
}

/// `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
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
        .description("Internal: reload provider-xai configuration when it changes."),
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
/// boot race.
pub async fn reconcile(iii: &IIIClient, cell: &ConfigCell) {
    on_config_change(iii, cell).await;
}

async fn on_config_change(iii: &IIIClient, cell: &ConfigCell) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(cell, cfg).await;
            tracing::info!("provider-xai configuration reloaded");
        }
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
        }
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
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
