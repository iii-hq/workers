//! Integration with the `configuration` worker — register the schema, fetch
//! the authoritative value at boot, and hot-reload it when it changes. Tier 1
//! (ConfigCell + targeted re-bind), mirroring `context-manager` /
//! `approval-gate`.
//!
//! Every field hot-reloads — nothing requires a restart:
//!
//! - `sweep_expression` is the one STRUCTURAL field (the cron binding for the
//!   pending-call sweep). On a change the handler re-binds the trigger live
//!   (register the new binding, then `unregister()` the old — a fail-safe
//!   overlap; the sweep is idempotent), keeping the [`Trigger`] handle in
//!   [`TriggerHandles`].
//! - Every OTHER field is a per-call tuning knob read from the live snapshot
//!   via [`Deps::cfg`](crate::deps::Deps::cfg); a change swaps the snapshot.
//!
//! `configuration` is a required boot dependency: a failed register/fetch
//! aborts startup.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::functions::sweep_pending::SWEEP_PENDING_ID;

/// Hot-swappable config snapshot shared with every handler.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "harness";
const CONFIG_FN_ID: &str = "harness::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the `harness` configuration schema. When `seed` is present its
/// value is installed as `initial_value`; otherwise the built-in default is
/// seeded only when nothing is stored yet (safe to call every boot).
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    discard_legacy_harness_config_if_needed(iii).await?;

    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Harness",
        "description": "Durable turn-loop settings: default turn cap and pending-call timeout, \
                        sub-agent depth/fan-out budgets, output-contract validation retries, the \
                        webhook-dedupe TTL, per-dependency RPC timeouts, stream coalescing, and \
                        the pending-sweep cron schedule.",
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

/// Read the live `harness` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
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

/// Returns `Ok(None)` when the entry does not exist (codes vary in case).
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// True when the stored value matches the pre-Rust harness entry (permissions
/// block and/or stale provider credentials).
fn is_legacy_harness_value(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("permissions") || obj.contains_key("providers")
}

/// Legacy keys present in a stored value (for the discard warning).
fn legacy_harness_keys(value: &Value) -> Vec<&'static str> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    if obj.contains_key("permissions") {
        keys.push("permissions");
    }
    if obj.contains_key("providers") {
        keys.push("providers");
    }
    keys
}

/// Discard a pre-Rust harness configuration entry so strict schema registration
/// can succeed. There is no `configuration::delete`; unlock a permissive schema,
/// replace the value with built-in defaults, then let the caller register the
/// real schema.
async fn discard_legacy_harness_config_if_needed(iii: &IIIClient) -> Result<(), String> {
    let Some(value) = try_get_config_value(iii).await? else {
        return Ok(());
    };
    if value.is_null() || !is_legacy_harness_value(&value) {
        return Ok(());
    }

    let dropped = legacy_harness_keys(&value);
    tracing::warn!(
        dropped_keys = ?dropped,
        "legacy pre-Rust harness configuration detected; discarding stored value and \
         re-seeding with built-in turn-loop defaults — provider credentials belong in the \
         llm-router entry, approval defaults belong in the approval-gate entry"
    );

    trigger_with_retry(
        iii,
        "configuration::register",
        json!({
            "id": CONFIG_ID,
            "name": "Harness",
            "description": "Legacy harness configuration placeholder (discarding pre-Rust entry).",
            "schema": {
                "type": "object",
                "additionalProperties": true
            },
        }),
    )
    .await?;

    trigger_with_retry(
        iii,
        "configuration::set",
        json!({
            "id": CONFIG_ID,
            "value": WorkerConfig::default().to_json(),
        }),
    )
    .await?;

    Ok(())
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Live handle for the one hot-reloadable trigger binding — the cron sweep.
pub struct TriggerHandles {
    pub sweep: std::sync::Mutex<Option<Trigger>>,
}

/// Best-effort binding: the cron trigger type always exists (engine built-in),
/// but a transient failure must not brick boot — it surfaces as a `None`
/// handle.
fn bind(iii: &IIIClient, trigger_type: &str, function_id: &str, config: Value) -> Option<Trigger> {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    }) {
        Ok(handle) => {
            tracing::info!(trigger_type, function_id, "trigger binding requested");
            Some(handle)
        }
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed");
            None
        }
    }
}

/// (Re)bind the cron pending-sweep from the current config.
pub fn bind_sweep(iii: &IIIClient, cfg: &WorkerConfig) -> Option<Trigger> {
    bind(
        iii,
        "cron",
        SWEEP_PENDING_ID,
        json!({ "expression": cfg.sweep_expression }),
    )
}

/// Store the freshly-registered handle, then unregister the old one
/// (register-new-then-unregister-old: a fail-safe overlap).
fn rebind_slot(slot: &std::sync::Mutex<Option<Trigger>>, new: Option<Trigger>) {
    let Some(new) = new else {
        return;
    };
    let old = slot.lock().unwrap_or_else(|p| p.into_inner()).replace(new);
    if let Some(old) = old {
        old.unregister();
    }
}

/// Internal `harness::on-config-change` trigger payload. The handler re-fetches
/// the authoritative configuration, so this carries only the (advisory)
/// configuration id; a struct keeps the request schema concrete.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `harness::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. `handles` holds the live cron `Trigger` the handler re-binds when
/// `sweep_expression` changes.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    handles: Arc<TriggerHandles>,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let handles_for_fn = handles.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let handles = handles_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &handles).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload harness from the authoritative configuration when it changes — \
             re-binds the cron pending-sweep on a sweep_expression change and swaps the per-call \
             tuning snapshot otherwise.",
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

/// Reload from the AUTHORITATIVE configuration. The caller-supplied trigger
/// payload is intentionally ignored: a direct call can never inject config.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, handles: &TriggerHandles) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let old = cell.read().await.clone();
    if old.boot_signature() != cfg.boot_signature() {
        rebind_slot(&handles.sweep, bind_sweep(iii, &cfg));
        tracing::info!("harness pending-sweep re-bound (sweep_expression changed)");
    }

    apply_config(cell, cfg).await;
    tracing::info!("harness configuration reloaded");
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
                    tracing::warn!(function_id, attempt, error = %last_err, "configuration RPC failed; retrying");
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
    fn is_legacy_harness_value_detects_old_entry_shape() {
        assert!(is_legacy_harness_value(&json!({
            "permissions": { "default_mode": "manual" }
        })));
        assert!(is_legacy_harness_value(&json!({
            "providers": { "anthropic": {} }
        })));
        assert!(!is_legacy_harness_value(&WorkerConfig::default().to_json()));
        assert!(!is_legacy_harness_value(&json!(null)));
    }

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.default_max_turns, 500);
        apply_config(
            &cell,
            WorkerConfig {
                default_max_turns: 99,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.default_max_turns, 99);
    }
}
