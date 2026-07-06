//! Integration with the `configuration` worker: register the schema, fetch
//! the authoritative value at boot, and hot-reload on change (Tier 1:
//! ConfigCell snapshot swap plus a targeted cron re-bind when
//! `prune_schedule` changes). Mirrors `approval-gate`.
//!
//! `configuration` is a REQUIRED boot dependency: a failed register/fetch
//! aborts startup.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler: take a
/// `read().await`, clone the inner `Arc` out, drop the lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "worktree";
const CONFIG_FN_ID: &str = "worktree::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the `worktree` configuration schema. A `seed` installs
/// `initial_value`; otherwise the built-in default is seeded only when no
/// stored value exists yet (safe to call every boot).
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Worktree",
        "description": "Git worktree lifecycle settings: the managed worktree root, \
                        branch prefix, prune schedule and expiry, land queue name, \
                        retry bound, and git/test timeouts.",
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

/// Read the live `worktree` configuration (already env-expanded by the
/// configuration worker).
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

/// `Ok(None)` when the entry does not exist; missing-entry codes vary in
/// case across engine versions, so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// The live prune cron binding, re-bindable on `prune_schedule` change.
/// Register the new binding first, then unregister the old (fail-safe
/// overlap).
#[derive(Clone, Default)]
pub struct CronSlot {
    inner: Arc<Mutex<Option<Trigger>>>,
}

impl CronSlot {
    pub fn rebind(&self, iii: &IIIClient, schedule: &str) {
        // `expression` is the binding key the cron trigger type consumes
        // (engine built-in and the standalone cron worker alike), matching
        // how the harness binds its own sweep.
        let new = match iii.register_trigger(RegisterTriggerInput {
            trigger_type: "cron".to_string(),
            function_id: "worktree::prune".to_string(),
            config: json!({ "expression": schedule }),
            metadata: None,
        }) {
            Ok(trigger) => {
                tracing::info!(schedule, "prune cron binding registered");
                trigger
            }
            Err(e) => {
                // Keep the working binding: swapping in a failed registration
                // would unregister the live cron and leave the sweep unbound.
                tracing::warn!(error = %e, schedule, "prune cron binding failed; keeping previous binding");
                return;
            }
        };
        let old = {
            let mut slot = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            slot.replace(new)
        };
        if let Some(old) = old {
            old.unregister();
        }
    }
}

/// Internal `worktree::on-config-change` trigger payload. The handler
/// re-fetches the authoritative configuration and ignores this payload, so
/// a direct call can never inject config.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `worktree::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. Registered here (not in `functions::register_all`) but still
/// pinned by `surface::catalog()`.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    cron: CronSlot,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    let cron_for_fn = cron.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            let cron = cron_for_fn.clone();
            async move {
                on_config_change(&engine, &cell, &cron).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the worktree worker from the authoritative \
             configuration when it changes — swaps the per-call snapshot and \
             re-binds the prune cron on a schedule change.",
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

/// Reload from the AUTHORITATIVE configuration; keep the previous snapshot
/// on any failure path.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, cron: &CronSlot) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: fetch failed; keeping previous configuration"
            );
            return;
        }
    };
    let previous_signature = cell.read().await.boot_signature();
    let structural = cfg.boot_signature() != previous_signature;
    let schedule = cfg.prune_schedule.clone();
    apply_config(cell, cfg).await;
    if structural {
        cron.rebind(iii, &schedule);
    }
    tracing::info!(structural, "worktree configuration reloaded");
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
                timeout_ms: None,
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

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.max_land_retries, 3);
        let tuned = WorkerConfig {
            max_land_retries: 9,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.max_land_retries, 9);
    }
}
