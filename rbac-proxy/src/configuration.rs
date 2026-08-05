//! Integration with the `configuration` worker — register the schema, fetch
//! the authoritative value at boot, and hot-reload it when it changes. Mirrors
//! the Tier-1 `ConfigCell + targeted rebuild` pattern of
//! [`approval-gate`](../../approval-gate/src/configuration.rs) /
//! [`context-manager`](../../context-manager/src/configuration.rs).
//!
//! `configuration` is a REQUIRED boot dependency: a failed register/fetch
//! aborts startup. The hot-reload tier split is the proxy's one structural
//! resource — the public listener:
//!
//! - `host` / `port` change → **rebind** the listener (last-good on bind
//!   failure: keep the previous listener *and* config).
//! - everything else (`engine_url`, `rbac.*`, `middleware_function_id`,
//!   `expose_worker_internals`) → **snapshot swap**; new connections pick it
//!   up, in-flight connections keep the boundaries they were admitted under.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::engine_overrides::CatalogCache;
use crate::server::ServerHandle;

/// Hot-swappable config snapshot shared with every connection. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets an upgrade take a `read().await`
/// and cheaply `clone()` the inner `Arc` out without holding the lock, while a
/// reload whole-snapshot replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const DEFAULT_CONFIG_ID: &str = "rbac-proxy";

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
const CONFIG_FN_ID: &str = "rbac-proxy::on-config-change";
const FUNCTIONS_AVAILABLE_FN_ID: &str = "rbac-proxy::on-functions-available";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the `rbac-proxy` configuration schema. When `seed` is present
/// (operator `--config`), its value is installed as `initial_value`.
/// Otherwise the built-in default — with `engine_url` defaulted to the control
/// connection's `--url` — is seeded only when no stored value exists yet
/// (re-registration preserves the stored value, so this is safe every boot).
pub async fn register_config(
    iii: &IIIClient,
    default_engine_url: &str,
    seed: Option<&WorkerConfig>,
) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "RBAC Proxy",
        "description": "Boundary-proxy settings: the public RBAC port, the trusted upstream engine URL, \
                        the RBAC contract (auth function, expose filters, registration hooks), middleware, \
                        and the worker-internals leak knob.",
        "schema": WorkerConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = default_config(default_engine_url).to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `rbac-proxy` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
pub async fn fetch_config(
    iii: &IIIClient,
    default_engine_url: &str,
) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(default_config(default_engine_url));
    }
    WorkerConfig::from_json(&value)
}

/// The built-in default, with `engine_url` defaulted to the control
/// connection's URL (spec: the data plane defaults to the same engine the
/// control connection used; a config override fronts a different engine).
fn default_config(default_engine_url: &str) -> WorkerConfig {
    WorkerConfig {
        engine_url: default_engine_url.to_string(),
        ..WorkerConfig::default()
    }
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
        .ok_or_else(|| format!("configuration `{config_entry}` not found", config_entry = config_id()))
}

/// `Ok(None)` when the entry does not exist. Engine missing-entry codes vary in
/// case, so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": config_id() })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Internal `rbac-proxy::on-config-change` trigger payload. The handler
/// re-fetches the authoritative config and ignores this advisory id.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. The handler re-fetches via `configuration::get` and ignores the
/// trigger payload, so a direct call can never inject config. Registered LAST
/// in boot so the handler closes over the fully-built server handle.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    server: Arc<ServerHandle>,
    default_engine_url: String,
) -> Result<(), Error> {
    let cell_fn = cell.clone();
    let iii_fn = iii.clone();
    let server_fn = server.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_fn.clone();
            let iii = iii_fn.clone();
            let server = server_fn.clone();
            let url = default_engine_url.clone();
            async move {
                on_config_change(&iii, &cell, &server, &url).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload rbac-proxy from the authoritative configuration — rebinds the public \
             listener on a host/port change (last-good on failure), else swaps the per-connection snapshot.",
        ),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
        namespace: iii.namespace(),
    })?;
    Ok(())
}

/// Reload from the AUTHORITATIVE configuration (the trigger payload is ignored).
async fn on_config_change(
    iii: &IIIClient,
    cell: &ConfigCell,
    server: &ServerHandle,
    default_engine_url: &str,
) {
    let new = match fetch_config(iii, default_engine_url).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: failed to fetch authoritative configuration; keeping previous config");
            return;
        }
    };

    let current = cell.read().await.clone();
    if current.requires_rebind(&new) {
        match server.rebind(&new.host, new.port).await {
            Ok(()) => {
                apply_config(cell, new).await;
                tracing::info!("rbac-proxy configuration reloaded (listener rebound)");
            }
            Err(e) => {
                tracing::error!(error = %e, "config-change: rebinding the public listener failed; keeping the previous listener and config (last-good)");
            }
        }
    } else {
        apply_config(cell, new).await;
        tracing::info!("rbac-proxy configuration reloaded (snapshot swap)");
    }
}

// ---------------------------------------------------------------------------
// Catalog-cache feed — proactively invalidate the discovery catalog when the
// engine's function set changes, so a freshly-registered (or removed) function
// is reflected sooner than the lazy TTL. Best-effort; the TTL is the backstop.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct FunctionsAvailableEvent {
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct FunctionsAvailableAck {
    pub ok: bool,
}

/// Register the internal `engine::functions-available` handler and bind it.
pub fn bind_catalog_refresh(iii: &Arc<IIIClient>, catalog: Arc<CatalogCache>) {
    let c = catalog.clone();
    iii.register_function(
        FUNCTIONS_AVAILABLE_FN_ID,
        RegisterFunction::new_async(move |_e: FunctionsAvailableEvent| {
            let c = c.clone();
            async move {
                c.invalidate().await;
                Ok::<FunctionsAvailableAck, Error>(FunctionsAvailableAck { ok: true })
            }
        })
        .description(
            "Internal: invalidate the discovery catalog cache when the engine's available function set changes.",
        ),
    );

    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: "engine::functions-available".to_string(),
        function_id: FUNCTIONS_AVAILABLE_FN_ID.to_string(),
        config: json!({}),
        metadata: None,
        namespace: iii.namespace(),
    }) {
        Ok(_) => tracing::info!("bound catalog-refresh trigger (engine::functions-available)"),
        Err(e) => {
            tracing::warn!(error = %e, "catalog-refresh trigger binding failed; relying on the TTL refresh")
        }
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
                timeout_ms: None,
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

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.port, 49200);
        apply_config(
            &cell,
            WorkerConfig {
                port: 50001,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.port, 50001);
    }

    #[test]
    fn default_config_uses_control_url() {
        let c = default_config("wss://remote:9000");
        assert_eq!(c.engine_url, "wss://remote:9000");
        assert_eq!(c.port, 49200); // other defaults intact
    }
}
