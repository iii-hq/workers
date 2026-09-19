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

pub const DEFAULT_CONFIG_ID: &str = "approval-gate";

/// The configuration entry this worker owns.
///
/// `III_CONFIG_NAME` when a supervisor set it, else the built-in name. A worker
/// that hardcodes its id turns that id into a global scarce name: two instances
/// share one entry and take turns overwriting it, and each write wakes both.
/// Being told which entry is its own is what lets them differ.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}
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
const HOOK_RETRY_INTERVAL_MS: u64 = 500;

/// Fixed `harness::hook::post-trigger` binding for `approval::filesystem-access-watch`
/// — only `shell::*` / `coder::*` dispatch results carry a `filesystem_access_request`
/// worth watching for. `fail_open` (unlike the pre_trigger gate): a
/// crashed/timed-out filesystem-access-watch must never turn an already-decided
/// function_result into a stuck call.
const FILESYSTEM_ACCESS_WATCH_FUNCTIONS: &[&str] = &["shell::*", "coder::*"];
const FILESYSTEM_ACCESS_WATCH_TIMEOUT_MS: u64 = 5_000;
const FILESYSTEM_ACCESS_WATCH_ON_ERROR: &str = "fail_open";

// Routed calls include engine metadata such as `_caller_worker_id`.
// The payload never controls which configuration entry is returned.
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ConfigurationIdentityRequest {}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct ConfigurationIdentityResponse {
    id: String,
}

/// Register the schema, seeding only when no stored value exists. Publish
/// the instance's identity in its namespace so UI policy edits cannot target
/// another project's configuration.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    iii.register_function(
        "approval-gate::configuration-id",
        RegisterFunction::new(|_request: ConfigurationIdentityRequest| {
            Ok::<_, Error>(ConfigurationIdentityResponse {
                id: config_id().to_string(),
            })
        })
        .description(
            "Returns this approval-gate instance's configuration entry ID, without its value.",
        )
        .metadata(json!({ "internal": true })),
    );
    let mut payload = json!({
        "id": config_id(),
        "name": "Approval Gate",
        "description": "Policy and decision surface settings: the deployment \
                        approval defaults (permission mode for new sessions \
                        and the auto-mode trust seed) and the agent permission \
                        rules.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    if should_seed_default_value(iii).await? {
        payload["initial_value"] = seed.cloned().unwrap_or_default().to_json();
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
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
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// Returns `Ok(None)` only for a missing entry. A missing service
/// (`function_not_found`) must propagate rather than authorize seeding.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Bind the fixed `harness::hook::pre-trigger` hook at worker startup.
pub fn bind_hook(iii: &IIIClient) {
    match iii.register_trigger(RegisterTriggerInput::new(
        "harness::hook::pre-trigger".to_string(),
        "approval::gate".to_string(),
        json!({
            "functions": HOOK_FUNCTIONS,
            "timeout_ms": HOOK_TIMEOUT_MS,
            "on_error": HOOK_ON_ERROR,
        }),
    )) {
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
/// `approval::filesystem-access-watch` at worker startup — beside `bind_hook`, same
/// best-effort discipline (a standalone deployment without the harness
/// still boots; a missing binding surfaces as a log, never an `Err` here).
pub fn bind_filesystem_access_watch_hook(iii: &IIIClient) {
    match iii.register_trigger(RegisterTriggerInput::new(
        "harness::hook::post-trigger".to_string(),
        "approval::filesystem-access-watch".to_string(),
        json!({
            "functions": FILESYSTEM_ACCESS_WATCH_FUNCTIONS,
            "timeout_ms": FILESYSTEM_ACCESS_WATCH_TIMEOUT_MS,
            "on_error": FILESYSTEM_ACCESS_WATCH_ON_ERROR,
        }),
    )) {
        Ok(_) => tracing::info!(
            trigger_type = "harness::hook::post-trigger",
            function_id = "approval::filesystem-access-watch",
            "trigger binding requested"
        ),
        Err(e) => tracing::warn!(
            trigger_type = "harness::hook::post-trigger",
            function_id = "approval::filesystem-access-watch",
            error = %e,
            "trigger binding failed (sibling absent?)"
        ),
    }
}

/// Retry hook bindings until the harness has registered the hook trigger
/// types. Approval-gate may start before harness; a one-shot registration in
/// that order fails asynchronously and silently leaves the gate detached.
pub fn retry_hook_bindings(iii: IIIClient) {
    tokio::spawn(async move {
        loop {
            let pre_trigger_ready = trigger_instance_count(&iii, "harness::hook::pre-trigger")
                .await
                .is_some_and(|count| count > 0);
            if !pre_trigger_ready {
                bind_hook(&iii);
            }

            let post_trigger_ready = trigger_instance_count(&iii, "harness::hook::post-trigger")
                .await
                .is_some_and(|count| count > 0);
            if !post_trigger_ready {
                bind_filesystem_access_watch_hook(&iii);
            }

            if pre_trigger_ready && post_trigger_ready {
                tracing::info!("approval-gate hook bindings confirmed");
                break;
            }

            tokio::time::sleep(Duration::from_millis(HOOK_RETRY_INTERVAL_MS)).await;
        }
    });
}

async fn trigger_instance_count(iii: &IIIClient, trigger_type: &str) -> Option<u64> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::info".to_string(),
            payload: json!({ "id": trigger_type }),
            action: None,
            timeout_ms: None,
        })
        .await
        .ok()?;
    response.get("instance_count").and_then(Value::as_u64)
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
        )
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
    ))?;
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

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: None,
                }
                .namespace("default"),
            )
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

/// `true` only when the error carries the configuration worker's standalone
/// `NOT_FOUND` entry code, identified by the outermost `remote error (<code>)` envelope code rather than a substring or token scan of the message, so
/// a compound code such as `RESOURCE_NOT_FOUND`/`STATEMENT_NOT_FOUND` or the
/// engine's lowercase missing-FUNCTION code `function_not_found` still
/// propagates as a failure instead of being read as "nothing stored yet".
fn is_not_found(error: &str) -> bool {
    match error.split_once("remote error (") {
        Some((_, rest)) => rest
            .split_once(')')
            .is_some_and(|(code, _)| code == "NOT_FOUND"),
        None => error.trim() == "NOT_FOUND",
    }
}

#[cfg(test)]
mod tests {
    /// Engine-injected caller metadata must not prevent resolving this worker's entry ID.
    #[test]
    fn configuration_identity_accepts_engine_caller_metadata() {
        for payload in [
            serde_json::json!({}),
            serde_json::json!({
                "_caller_worker_id": "00000000-0000-4000-8000-000000000002"
            }),
        ] {
            serde_json::from_value::<super::ConfigurationIdentityRequest>(payload)
                .expect("routed identity requests accept engine metadata");
        }
    }

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
