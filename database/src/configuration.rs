//! Integration with the `configuration` worker — register, fetch, and hot-reload
//! the `database` configuration entry.

use crate::config::WorkerConfig;
use crate::handlers::AppState;
use crate::pool::{self, Pool};
use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest, III};
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
pub async fn register_config(iii: &III, seed: Option<&WorkerConfig>) -> Result<(), String> {
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
pub async fn fetch_config(iii: &III) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &III) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn get_config_value(iii: &III) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))
}

/// Returns `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_config_value(iii: &III) -> Result<Option<Value>, String> {
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

/// Replace in-memory pools with freshly built ones from `cfg`.
pub async fn apply_config(state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
    let new_pools = build_pools(&cfg).await?;
    let mut guard = state.pools.write().await;
    *guard = new_pools;
    Ok(())
}

/// Register the internal config-change handler and bind a `configuration` trigger.
pub fn register_config_trigger(iii: &III, state: AppState) -> Result<(), IIIError> {
    let st = state.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |payload: Value| {
            let st = st.clone();
            async move {
                on_config_change(&st, payload).await;
                Ok::<Value, IIIError>(json!({ "ok": true }))
            }
        })
        .description("Internal: reload connection pools when the database configuration changes."),
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

async fn on_config_change(state: &AppState, payload: Value) {
    let new_value = match payload.get("new_value") {
        Some(v) if !v.is_null() => v.clone(),
        _ => {
            tracing::warn!("configuration event missing new_value; skipping pool reload");
            return;
        }
    };
    match WorkerConfig::from_json(&new_value) {
        Ok(cfg) => match apply_config(state, cfg).await {
            Ok(()) => tracing::info!("database pools reloaded after configuration change"),
            Err(e) => tracing::error!(
                error = %e,
                "failed to rebuild pools after configuration change; keeping previous pools"
            ),
        },
        Err(e) => tracing::error!(
            error = %e,
            "invalid configuration payload; keeping previous pools"
        ),
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
