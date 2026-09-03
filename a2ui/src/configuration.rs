//! Path B integration with the `configuration` worker.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "a2ui";
pub const CONFIG_FN_ID: &str = "a2ui::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Advisory id only. The handler always re-fetches authoritative state.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "A2UI",
        "description": "A2UI composer routing, correction budget, per-session surface limits, and Console action forwarding.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": CONFIG_ID },
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

pub fn register_config_trigger(iii: &IIIClient, cell: ConfigCell) -> Result<(), Error> {
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(cfg) => {
                        apply_config(&cell, cfg).await;
                        tracing::info!("A2UI configuration reloaded");
                    }
                    Err(error) => tracing::error!(%error, "failed to reload A2UI configuration"),
                }
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload the A2UI worker from authoritative configuration after an update.",
        ),
    );
    iii.register_trigger(RegisterTriggerInput::new(
        "configuration",
        CONFIG_FN_ID,
        json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"]
        }),
    ))?;
    Ok(())
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    Ok(try_get_config_value(iii)
        .await?
        .is_none_or(|value| value.is_null()))
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))
}

async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({"id": CONFIG_ID})).await {
        Ok(response) => Ok(response.get("value").cloned()),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_not_found(error: &str) -> bool {
    error.to_ascii_uppercase().contains("NOT_FOUND")
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_error = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.into(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: None,
                }
                .namespace("default"),
            )
            .await
        {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error.to_string();
                if is_not_found(&last_error) {
                    return Err(last_error);
                }
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * u64::from(attempt),
                    ))
                    .await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_entry_detection_is_case_insensitive() {
        assert!(is_not_found("remote NOT_FOUND"));
        assert!(is_not_found("statement_not_found"));
        assert!(!is_not_found("timed out"));
    }

    #[tokio::test]
    async fn config_snapshot_swaps() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        apply_config(
            &cell,
            WorkerConfig {
                max_surfaces_per_session: 3,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.max_surfaces_per_session, 3);
    }
}
