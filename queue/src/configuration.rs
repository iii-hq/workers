//! Configuration worker integration.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::adapter::SwappableAdapter;
use crate::adapters::builtin::BuiltinAdapter;
use crate::boot::{ApplyLock, ConfigCell};
use crate::config::QueueConfig;
use crate::trigger::{IiiInvoker, QueueTriggerHandler};

pub const CONFIG_ID: &str = "queue";
pub const CONFIG_FN_ID: &str = "queue::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

pub fn new_cell(config: QueueConfig) -> ConfigCell {
    Arc::new(tokio::sync::RwLock::new(Arc::new(config.normalized())))
}

pub async fn register_config(iii: &IIIClient, seed: Option<&QueueConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Queue",
        "description": "Durable queue worker settings: in-process/file-backed transport persistence.",
        "schema": QueueConfig::json_schema(),
    });
    if should_seed_initial_value(iii).await? {
        let seed = seed.cloned().unwrap_or_default().normalized();
        payload["initial_value"] = seed.to_json();
    }
    trigger_with_retry(
        iii,
        "configuration::register",
        payload,
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<QueueConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => QueueConfig::from_json(&value).map(|c| c.normalized()),
        _ => {
            tracing::info!("no `{CONFIG_ID}` configuration value stored; using built-in default");
            Ok(QueueConfig::default())
        }
    }
}

pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    adapter: Arc<SwappableAdapter>,
    trigger_handler: QueueTriggerHandler,
    config: ConfigCell,
    apply_lock: ApplyLock,
) -> Result<(), Error> {
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let engine = engine.clone();
            let adapter = adapter.clone();
            let trigger_handler = trigger_handler.clone();
            let config = config.clone();
            let apply_lock = apply_lock.clone();
            async move {
                on_config_change(engine, adapter, trigger_handler, config, apply_lock).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload queue configuration from the authoritative store."),
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

async fn on_config_change(
    iii: Arc<IIIClient>,
    adapter: Arc<SwappableAdapter>,
    trigger_handler: QueueTriggerHandler,
    config: ConfigCell,
    apply_lock: ApplyLock,
) {
    let _guard = apply_lock.lock().await;

    let next = match fetch_config(&iii).await {
        Ok(config) => config.normalized(),
        Err(err) => {
            tracing::error!(error = %err, "queue config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let old = config.read().await.clone();
    if swap_needed(&old, &next) {
        let registrations = trigger_handler.registrations().await;
        let new_store = match crate::boot::build_store(&next).await {
            Ok(store) => store,
            Err(err) => {
                tracing::error!(error = %err, "queue config-change: failed to build replacement store; keeping previous config");
                return;
            }
        };
        let new_adapter = Arc::new(BuiltinAdapter::new(
            new_store,
            Arc::new(IiiInvoker::new(iii.clone())),
        ));
        trigger_handler.shutdown().await;
        adapter.replace(new_adapter).await;
        *config.write().await = Arc::new(next);

        for registration in registrations {
            if let Err(err) = trigger_handler.register_subscriber(registration).await {
                tracing::error!(error = %err, "queue config-change: failed to restart subscriber");
            }
        }
        tracing::info!("queue transport hot-swapped after configuration change");
        return;
    }

    *config.write().await = Arc::new(next);
    tracing::info!("queue configuration reloaded without transport swap");
}

pub fn swap_needed(old: &QueueConfig, new: &QueueConfig) -> bool {
    effective_adapter(old) != effective_adapter(new)
}

fn effective_adapter(config: &QueueConfig) -> (String, Value) {
    (
        config.effective_adapter_name().to_string(),
        config
            .adapter
            .as_ref()
            .and_then(|adapter| adapter.config.clone())
            .unwrap_or(Value::Null),
    )
}

async fn should_seed_initial_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => Ok(false),
        _ => Ok(true),
    }
}

async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(
        iii,
        "configuration::get",
        json!({ "id": CONFIG_ID }),
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
        {
            Ok(value) => return Ok(value),
            Err(err) => {
                last_err = err.to_string();
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
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeAck {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeRequest {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdapterEntry;

    #[test]
    fn swap_needed_ignores_implicit_builtin_default() {
        let old = QueueConfig::default();
        let new = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "builtin".to_string(),
                config: None,
            }),
        };
        assert!(!swap_needed(&old, &new));
    }

    #[test]
    fn swap_needed_when_adapter_config_changes() {
        let old = QueueConfig::default();
        let new = QueueConfig {
            adapter: Some(AdapterEntry {
                name: "builtin".to_string(),
                config: Some(json!({
                    "store_method": "file_based",
                    "file_path": "./data/queue"
                })),
            }),
        };
        assert!(swap_needed(&old, &new));
    }
}
