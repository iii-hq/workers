//! Integration with the `configuration` worker — register, fetch, and hot-reload
//! the `database` configuration entry.

use crate::config::WorkerConfig;
use crate::handlers::AppState;
use crate::pool::{self, Pool};
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const CONFIG_ID: &str = "database";
const CONFIG_FN_ID: &str = "database::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register the `database` configuration schema with the configuration worker.
/// When `seed` is present, its value is installed as `initial_value`. Otherwise,
/// built-in defaults are seeded only when no stored value exists yet.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Database",
        "description": "Connection pools for PostgreSQL, MySQL, and SQLite.",
        "schema": WorkerConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `database` configuration (env-expanded by the configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
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

/// Returns `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Build connection pools for every configured database.
pub async fn build_pools(cfg: &WorkerConfig) -> Result<HashMap<String, Pool>, String> {
    let mut pools = HashMap::new();
    for (name, db) in &cfg.databases {
        let p = pool::build(name, db)
            .await
            .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))?;
        tracing::info!(db = %name, driver = ?p.driver(), "pool ready");
        pools.insert(name.clone(), p);
    }
    Ok(pools)
}

pub async fn apply_config(state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
    let new_pools = build_pools(&cfg).await?;
    // Swap pools and the config snapshot inside one critical section (pools
    // lock first, then config) so a concurrent reader never observes new
    // pools paired with the old config or vice-versa. A failed build above
    // leaves both untouched.
    let mut pools_guard = state.pools.write().await;
    let mut config_guard = state.config.write().await;
    *pools_guard = new_pools;
    *config_guard = cfg;
    Ok(())
}

/// Event delivered to the internal `database::on-config-change` handler. A struct
/// (not `Value`) keeps the request schema concrete; the handler re-fetches the
/// configuration id; unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    /// Schema-only: kept to publish a typed request schema; the handler ignores it.
    #[serde(default)]
    #[allow(dead_code)]
    pub id: Option<String>,
}

/// Ack returned by the internal `database::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration` trigger.
pub fn register_config_trigger(iii: &IIIClient, state: AppState) -> Result<(), Error> {
    let st = state.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let st = st.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &st).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload connection pools from the authoritative configuration when it changes.",
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
        namespace: iii.namespace(),
    })?;
    Ok(())
}

/// Reload pools from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `database::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller replace the live connection pools
/// (e.g. point them at an attacker-controlled database) without updating
/// persisted state. Re-fetch the stored value via `configuration::get` instead.
async fn on_config_change(iii: &IIIClient, state: &AppState) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous pools"
            );
            return;
        }
    };
    match apply_config(state, cfg).await {
        Ok(()) => tracing::info!("database pools reloaded after configuration change"),
        Err(e) => tracing::error!(
            error = %e,
            "failed to rebuild pools after configuration change; keeping previous pools"
        ),
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
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_err,
                        "configuration RPC failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}
