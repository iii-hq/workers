//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors [`context-manager`](../../context-manager/src/configuration.rs) /
//! [`session-manager`](../../session-manager/src/configuration.rs).
//!
//! Every configuration field hot-reloads via a snapshot swap — nothing
//! requires a restart. The harness `pre_trigger` hook binding is fixed at
//! worker startup (consult on all calls, fail closed).
//!
//! `configuration` is a REQUIRED boot dependency: a failed register/fetch
//! aborts startup (the gate must run on a known, authoritative policy
//! surface, never a guessed one).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a
/// `read().await` and `clone()` the inner `Arc` out (a cheap refcount
/// bump) without holding the lock across its work, while `apply_config`
/// whole-snapshot replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "approval-gate";
const CONFIG_FN_ID: &str = "approval::on-config-change";
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the
/// attempt number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Fixed `harness::hook::pre-trigger` binding — approval-gate consults on
/// every function call.
const HOOK_FUNCTIONS: &[&str] = &["*"];
const HOOK_TIMEOUT_MS: u64 = 5_000;
const HOOK_ON_ERROR: &str = "fail_closed";

/// Fixed `harness::hook::post-trigger` binding for `approval::grant-watch`
/// — only `shell::*` / `coder::*` dispatch results carry a `grant_hint`
/// worth watching for. `fail_open` (unlike the pre_trigger gate): a
/// crashed/timed-out grant-watch must never turn an already-decided
/// function_result into a stuck call.
const GRANT_WATCH_FUNCTIONS: &[&str] = &["shell::*", "coder::*"];
const GRANT_WATCH_TIMEOUT_MS: u64 = 5_000;
const GRANT_WATCH_ON_ERROR: &str = "fail_open";

/// Register the `approval-gate` configuration schema with the
/// configuration worker. When `seed` is present, its value is installed
/// as `initial_value`. Otherwise, the built-in default is seeded only
/// when no stored value exists yet (re-registration preserves the stored
/// value, so this is safe to call every boot).
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Approval Gate",
        "description": "Policy and decision surface settings: the deployment \
                        approval defaults (permission mode for new sessions \
                        and the auto-mode trust seed) and the agent permission \
                        rules.",
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

/// Read the live `approval-gate` configuration (env-expanded by the
/// configuration worker — `from_json` does NOT re-expand).
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

/// Returns `Ok(None)` when the entry does not exist. The engine's
/// missing-entry codes vary in case (`function_not_found`,
/// `STATEMENT_NOT_FOUND`, `NOT_FOUND`), so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Bind the fixed `harness::hook::pre-trigger` hook at worker startup.
pub fn bind_hook(iii: &IIIClient) {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "harness::hook::pre-trigger".to_string(),
        function_id: "approval::gate".to_string(),
        config: json!({
            "functions": HOOK_FUNCTIONS,
            "timeout_ms": HOOK_TIMEOUT_MS,
            "on_error": HOOK_ON_ERROR,
        }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(
            trigger_type = "harness::hook::pre-trigger",
            function_id = "approval::gate",
            "trigger binding requested"
        ),
        Err(e) => tracing::warn!(
            trigger_type = "harness::hook::pre-trigger",
            function_id = "approval::gate",
            error = %e,
            "trigger binding failed (sibling absent?)"
        ),
    }
}

/// Bind the fixed `harness::hook::post-trigger` hook for
/// `approval::grant-watch` at worker startup — beside `bind_hook`, same
/// best-effort discipline (a standalone deployment without the harness
/// still boots; a missing binding surfaces as a log, never an `Err` here).
pub fn bind_grant_watch_hook(iii: &IIIClient) {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "harness::hook::post-trigger".to_string(),
        function_id: "approval::grant-watch".to_string(),
        config: json!({
            "functions": GRANT_WATCH_FUNCTIONS,
            "timeout_ms": GRANT_WATCH_TIMEOUT_MS,
            "on_error": GRANT_WATCH_ON_ERROR,
        }),
        metadata: None,
    }) {
        Ok(_) => tracing::info!(
            trigger_type = "harness::hook::post-trigger",
            function_id = "approval::grant-watch",
            "trigger binding requested"
        ),
        Err(e) => tracing::warn!(
            trigger_type = "harness::hook::post-trigger",
            function_id = "approval::grant-watch",
            error = %e,
            "trigger binding failed (sibling absent?)"
        ),
    }
}

/// Internal `approval::on-config-change` trigger payload. The handler
/// re-fetches the authoritative configuration, so this carries only the
/// (advisory) configuration id; a struct (not `Value`) keeps the request
/// schema concrete and unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `approval::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. The handler re-fetches via `configuration::get` and ignores the
/// trigger payload, so a direct call can never inject config.
pub fn register_config_trigger(iii: &IIIClient, cell: ConfigCell) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload approval-gate from the authoritative configuration when it \
             changes — swaps the per-call snapshot (timeouts + approval defaults).",
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

/// Reload from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `approval::on-config-change` is a discoverable bus function, so
/// trusting `payload.new_value` would let any caller inject arbitrary
/// config without updating persisted state. Re-fetch the stored value via
/// `configuration::get` instead.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous config"
            );
            return;
        }
    };

    apply_config(cell, cfg).await;
    tracing::info!("approval-gate configuration reloaded");
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
    use crate::types::PermissionMode;

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.default_mode, PermissionMode::Manual);

        let tuned = WorkerConfig {
            default_mode: PermissionMode::Full,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.default_mode, PermissionMode::Full);
    }
}
