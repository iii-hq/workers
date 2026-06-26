//! Integration with the `configuration` worker — all fields hot-reload
//! without restart, including `bot_token`.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::deps::Deps;

pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "telegram-bot";
const CONFIG_FN_ID: &str = "telegram-bot::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

fn config_rpc_timeout_ms(seed: Option<&WorkerConfig>) -> u64 {
    seed.map(|s| s.timeout_ms)
        .unwrap_or_else(|| WorkerConfig::default().timeout_ms)
}

pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Telegram Worker",
        "description": "Telegram bot bridge: bot token, updates adapter (polling/webhook), verbosity, default model, and harness send options.",
        "schema": WorkerConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_with_retry(
        iii,
        "configuration::register",
        payload,
        config_rpc_timeout_ms(seed),
    )
    .await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    fetch_config_with_timeout(iii, WorkerConfig::default().timeout_ms).await
}

async fn fetch_config_with_timeout(
    iii: &IIIClient,
    timeout_ms: u64,
) -> Result<WorkerConfig, String> {
    let value = try_get_config_value(iii, timeout_ms)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii, WorkerConfig::default().timeout_ms).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn try_get_config_value(iii: &IIIClient, timeout_ms: u64) -> Result<Option<Value>, String> {
    match trigger_with_retry(
        iii,
        "configuration::get",
        json!({ "id": CONFIG_ID }),
        timeout_ms,
    )
    .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Apply a validated config snapshot. Returns false when rejected (empty token).
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) -> bool {
    if let Err(reason) = cfg.validate() {
        tracing::warn!(reason = %reason, "config reload rejected; keeping previous config");
        return false;
    }
    let rotated = {
        let prev = cell.read().await;
        prev.bot_token != cfg.bot_token
    };
    if rotated {
        tracing::info!("bot_token rotated (value not logged)");
    }
    *cell.write().await = Arc::new(cfg);
    true
}

pub fn register_config_trigger(
    iii: &IIIClient,
    cell: ConfigCell,
    deps: Arc<Deps>,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    let deps_for_fn = deps.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            let deps = deps_for_fn.clone();
            async move {
                on_config_change(&engine, &cell, &deps).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description(
            "Internal: reload telegram-bot configuration from the authoritative store on change.",
        ),
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

async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, deps: &Arc<Deps>) {
    let prev = cell.read().await.clone();
    let timeout_ms = prev.timeout_ms;
    let cfg = match fetch_config_with_timeout(iii, timeout_ms).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous config"
            );
            return;
        }
    };
    if apply_config(cell, cfg.clone()).await {
        crate::ingress::apply_config_change(deps, &prev, &cfg).await;
        if let Err(e) = crate::clients::telegram::set_my_commands(deps).await {
            tracing::warn!(error = %e, "failed to refresh bot commands after config reload");
        }
        tracing::info!("telegram-bot configuration reloaded");
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

#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct ConfigChangeAck {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeRequest {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apply_config_swaps_all_fields() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        let next = WorkerConfig {
            bot_token: "1:token".into(),
            verbosity: crate::config::Verbosity::Debug,
            timeout_ms: 500,
            ..WorkerConfig::default()
        };
        assert!(apply_config(&cell, next).await);
        let snap = cell.read().await.clone();
        assert_eq!(snap.bot_token, "1:token");
        assert_eq!(snap.verbosity, crate::config::Verbosity::Debug);
        assert_eq!(snap.timeout_ms, 500);
    }

    #[tokio::test]
    async fn apply_config_rejects_empty_token_keeps_previous() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig {
            bot_token: "keep:me".into(),
            ..WorkerConfig::default()
        })));
        assert!(!apply_config(&cell, WorkerConfig::default()).await);
        assert_eq!(cell.read().await.bot_token, "keep:me");
    }
}
