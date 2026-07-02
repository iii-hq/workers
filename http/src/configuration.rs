//! Integration with the builtin `configuration` worker.
//!
//! The worker registers its config schema under the id `http` (what the console
//! renders as an editable form), reads the authoritative value before binding,
//! and hot-applies `configuration:updated` events onto a shared [`ConfigCell`].
//!
//! ## What reloads live vs. restart-only
//!
//! The request handler reads the config from the cell **per request**, so the
//! fields it consumes hot-reload without a restart:
//!
//! - `middleware` — the global middleware chain.
//! - `default_timeout` — the per-request invocation, condition, middleware, and
//!   streaming-drain timeouts read inside the handler.
//!
//! The CORS layer, the outer tower `TimeoutLayer` (the 504 wrapper, built from
//! `default_timeout`), and the `ConcurrencyLimitLayer` are baked into the axum
//! `Router` at build time, but the server holds it behind a swappable
//! [`crate::server::HotRouter`] (Phase A). On a **same-address** config change,
//! [`on_config_change`] rebuilds those layers via
//! [`crate::server::rebuild_layers`], so `cors` / `default_timeout` /
//! `concurrency_request_limit` also take effect live without dropping the
//! listener.
//!
//! Only `host` / `port` remain **restart-only**: rebinding the TCP listener to a
//! new address is a separate later phase (Phase B). A host/port change is
//! applied to the cell (so a later restart picks it up) and logged with a
//! warning; the running listener stays on the old address.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::config::RestApiConfig;
use crate::server::{self, RouterCell};

/// Shared, swappable config snapshot. The handler reads it per-request so
/// `middleware`/`default_timeout` changes take effect without a restart.
pub type ConfigCell = Arc<RwLock<Arc<RestApiConfig>>>;

pub const CONFIG_ID: &str = "http";
const CONFIG_FN_ID: &str = "http::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
/// Upper bound on every `configuration::*` bus call (mirrors the engine's
/// `CONFIG_BUS_TIMEOUT` of 10s): a hung provider must not wedge boot or reload.
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

/// Wrap a config in a fresh cell (used at boot).
pub fn new_cell(config: RestApiConfig) -> ConfigCell {
    Arc::new(RwLock::new(Arc::new(config)))
}

/// Register the `http` configuration entry: schema + metadata refresh on every
/// boot; `initial_value` (the `--config` seed, or built-in defaults) is included
/// only when nothing is stored yet, so runtime edits survive restarts.
pub async fn register_config(iii: &IIIClient, seed: Option<&RestApiConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "HTTP",
        "description": "HTTP server settings — host/port binding, CORS, request timeout, concurrency limit, and global middleware.",
        "schema": RestApiConfig::json_schema(),
    });
    if should_seed_initial_value(iii).await? {
        let seed = seed.cloned().unwrap_or_default().normalized();
        payload["initial_value"] = seed.to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload, CONFIG_BUS_TIMEOUT_MS).await?;
    Ok(())
}

/// Read the live configuration value. A missing/null value falls back to the
/// built-in default; a malformed stored value is an error so callers keep their
/// previous config.
pub async fn fetch_config(iii: &IIIClient) -> Result<RestApiConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => RestApiConfig::from_json(&value),
        _ => {
            tracing::info!("no `{CONFIG_ID}` configuration value stored; using built-in default");
            Ok(RestApiConfig::default())
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

/// Swap the live config snapshot. Returns false when the candidate is rejected
/// (then the previous config is kept). Restart-only field changes are applied to
/// the cell but logged: the running listener/router does not pick them up.
pub async fn apply_config(cell: &ConfigCell, cfg: RestApiConfig) -> bool {
    if let Err(reason) = cfg.validate() {
        tracing::warn!(reason = %reason, "config reload rejected; keeping previous config");
        return false;
    }
    {
        let current = cell.read().await;
        warn_restart_only_changes(&current, &cfg);
    }
    *cell.write().await = Arc::new(cfg);
    true
}

/// Log a warning when host/port changes, so an operator knows the running
/// listener has not picked the change up (host/port rebind is Phase B). CORS /
/// timeout / concurrency changes are NOT warned here: on a same-address change
/// they hot-reload via [`crate::server::rebuild_layers`].
fn warn_restart_only_changes(current: &RestApiConfig, next: &RestApiConfig) {
    if current.host != next.host || current.port != next.port {
        tracing::warn!(
            old = %format!("{}:{}", current.host, current.port),
            new = %format!("{}:{}", next.host, next.port),
            "http: host/port change requires a worker restart; the listener stays on the old address"
        );
    }
}

/// Register `http::on-config-change` and subscribe it to `configuration:updated`
/// events for the `http` entry. The handler ignores the trigger payload and
/// re-fetches the authoritative value (the bus function is discoverable, so a
/// caller-supplied payload must not be trusted to repoint config).
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    router: RouterCell,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let router_for_fn = router.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let cell = cell_for_fn.clone();
            let router = router_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &router).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload http configuration from the authoritative store on change."),
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

async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, router: &RouterCell) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            // Capture the address BEFORE the swap so we can tell a same-address
            // change (rebuild layers live) from a host/port change (restart-only).
            let old_addr = {
                let current = cell.read().await;
                format!("{}:{}", current.host, current.port)
            };
            let new_addr = format!("{}:{}", cfg.host, cfg.port);

            if apply_config(cell, cfg).await {
                if old_addr == new_addr {
                    // Same address: rebuild the CORS/timeout/concurrency layers
                    // from the now-current snapshot and swap them into the live
                    // router. The listener keeps running.
                    let snapshot = cell.read().await.clone();
                    server::rebuild_layers(router, &snapshot).await;
                }
                // A host/port change was already warned in warn_restart_only_changes;
                // Phase B will rebind the listener.
                tracing::info!("http configuration reloaded");
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config")
        }
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeAck {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeRequest {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MiddlewareConfig;

    fn cell(cfg: RestApiConfig) -> ConfigCell {
        new_cell(cfg)
    }

    #[tokio::test]
    async fn apply_config_swaps_hot_fields() {
        let c = cell(RestApiConfig::default());
        let next = RestApiConfig {
            default_timeout: 1234,
            middleware: vec![MiddlewareConfig {
                function_id: "mw::block".into(),
                phase: "preHandler".into(),
                priority: 0,
            }],
            ..RestApiConfig::default()
        };
        assert!(apply_config(&c, next).await);
        let snap = c.read().await.clone();
        assert_eq!(snap.default_timeout, 1234);
        assert_eq!(snap.middleware.len(), 1);
        assert_eq!(snap.middleware[0].function_id, "mw::block");
    }
}
