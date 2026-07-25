//! Integration with the builtin `configuration` worker.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::boot::{self, BootParts, ConfigCell, SchedulerCell};
use crate::config::CronConfig;
use crate::locks;
use crate::scheduler::Scheduler;

pub const CONFIG_ID: &str = "cron";
const CONFIG_FN_ID: &str = "cron::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

pub async fn register_config(iii: &IIIClient, seed: Option<&CronConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Cron",
        "description": "Cron scheduler settings - lock backend for multi-instance mutual exclusion (local or redis).",
        "schema": CronConfig::json_schema(),
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

pub async fn fetch_config(iii: &IIIClient) -> Result<CronConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => CronConfig::from_json(&value),
        _ => {
            tracing::info!("no `{CONFIG_ID}` configuration value stored; using built-in default");
            Ok(CronConfig::default())
        }
    }
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

pub fn register_config_trigger(iii: &Arc<IIIClient>, parts: BootParts) -> Result<(), Error> {
    let engine = iii.clone();
    let parts_for_fn = parts.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let engine = engine.clone();
            let parts = parts_for_fn.clone();
            async move {
                on_config_change(&engine, &parts).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload cron configuration from the authoritative store on change."),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
        namespace: iii.namespace(),
    })?;
    Ok(())
}

async fn on_config_change(iii: &Arc<IIIClient>, parts: &BootParts) {
    let _guard = parts.apply_lock.lock().await;

    let next = match fetch_config(iii).await {
        Ok(cfg) => cfg.normalized(),
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let current = parts.config.read().await.clone();
    if !swap_needed(&current, &next) {
        *parts.config.write().await = next;
        tracing::info!("cron configuration reloaded without lock-backend swap");
        return;
    }

    if let Err(e) = swap_scheduler(iii, &parts.scheduler, &parts.config, next).await {
        tracing::error!(error = %e, "cron lock-backend swap failed; keeping previous scheduler");
    }
}

async fn swap_scheduler(
    iii: &Arc<IIIClient>,
    scheduler_cell: &SchedulerCell,
    config_cell: &ConfigCell,
    next: CronConfig,
) -> anyhow::Result<()> {
    let lock = locks::build_lock(&next).await?;
    let old = scheduler_cell.read().await.clone();
    let specs = old.job_specs().await;
    let new_scheduler = Arc::new(Scheduler::new(lock, boot::sdk_invoker(iii.clone())));

    old.shutdown().await;
    for spec in specs {
        if let Err(e) = new_scheduler.register(spec.clone()).await {
            tracing::error!(
                trigger_id = %spec.trigger_id,
                error = %e,
                "failed to re-register cron job after lock-backend swap"
            );
        }
    }

    *scheduler_cell.write().await = new_scheduler;
    *config_cell.write().await = next;
    tracing::info!("cron configuration reloaded with lock-backend swap");
    Ok(())
}

fn swap_needed(current: &CronConfig, next: &CronConfig) -> bool {
    current.effective_adapter_name() != next.effective_adapter_name()
        || current.adapter.as_ref().and_then(|a| a.config.clone())
            != next.adapter.as_ref().and_then(|a| a.config.clone())
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
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
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

    #[test]
    fn swap_needed_only_when_adapter_changes() {
        let local: CronConfig = serde_yaml::from_str("{adapter: {name: local}}").unwrap();
        let redis: CronConfig = serde_yaml::from_str("{adapter: {name: redis}}").unwrap();
        assert!(!swap_needed(&local, &local));
        assert!(swap_needed(&local, &redis));
        assert!(
            !swap_needed(&CronConfig::default(), &local),
            "default IS local"
        );
    }
}
