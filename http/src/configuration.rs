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
//! A `host` / `port` change **rebinds** the listener live (Phase B): the new
//! address is bound first (a failed bind keeps the old server), the new server
//! is spawned over the shared router cell + state, and the old server is then
//! gracefully shut down. See [`on_config_change`] and the shared
//! [`crate::server::ServerControlCell`].
//!
//! [`on_config_change`] is serialized end-to-end by an [`ApplyLock`]: the SDK
//! dispatches each function invocation via `tokio::spawn`, so overlapping
//! `configuration:updated` events could otherwise interleave their
//! [`ConfigCell`] / [`crate::server::ServerControlCell`] mutations. Mirrors
//! the engine's `apply_lock` (`api_core.rs`).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::config::RestApiConfig;
use crate::server::{self, HotRouter, ServerControlCell};

/// Shared, swappable config snapshot. The handler reads it per-request so
/// `middleware`/`default_timeout` changes take effect without a restart.
pub type ConfigCell = Arc<RwLock<Arc<RestApiConfig>>>;

/// Serializes concurrent `on_config_change` runs. The SDK dispatches each
/// function invocation via `tokio::spawn`, so two `http::on-config-change`
/// invocations (rapid configuration edits) can run concurrently; without this
/// lock they could re-fetch, validate, and swap the [`ConfigCell`] /
/// [`ServerControlCell`] out of order, leaving the config cell disagreeing
/// with the actually-bound listener. Mirrors the engine's `apply_lock`
/// (`engine/src/workers/rest_api/api_core.rs`).
pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;

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
    trigger_with_retry(
        iii,
        "configuration::register",
        payload,
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await?;
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
/// (then the previous config is kept). Used on the same-address path; the
/// address-change (rebind) path swaps the cell itself, after a successful bind.
pub async fn apply_config(cell: &ConfigCell, cfg: RestApiConfig) -> bool {
    if let Err(reason) = cfg.validate() {
        tracing::warn!(reason = %reason, "config reload rejected; keeping previous config");
        return false;
    }
    *cell.write().await = Arc::new(cfg);
    true
}

/// Register `http::on-config-change` and subscribe it to `configuration:updated`
/// events for the `http` entry. The handler ignores the trigger payload and
/// re-fetches the authoritative value (the bus function is discoverable, so a
/// caller-supplied payload must not be trusted to repoint config).
///
/// `hot_router` (its `inner` is the shared router cell) rebuilds layers on a
/// same-address change and is reused to spawn a rebound server; `control` tracks
/// the currently-running server so a host/port change can replace it.
/// `apply_lock` serializes overlapping invocations of the handler (see
/// [`ApplyLock`]).
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    hot_router: HotRouter,
    control: ServerControlCell,
    apply_lock: ApplyLock,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let hot_router_for_fn = hot_router.clone();
    let control_for_fn = control.clone();
    let apply_lock_for_fn = apply_lock.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let cell = cell_for_fn.clone();
            let hot_router = hot_router_for_fn.clone();
            let control = control_for_fn.clone();
            let apply_lock = apply_lock_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &hot_router, &control, &apply_lock).await;
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
        namespace: iii.namespace(),
    })?;
    Ok(())
}

async fn on_config_change(
    iii: &IIIClient,
    cell: &ConfigCell,
    hot_router: &HotRouter,
    control: &ServerControlCell,
    apply_lock: &ApplyLock,
) {
    // Serialize the whole re-fetch -> validate -> swap-cell -> (rebuild_layers
    // | rebind) sequence: the SDK dispatches each invocation via `tokio::spawn`,
    // so two `configuration:updated` events can run this function concurrently.
    // Without this, two overlapping edits could interleave their cell/server
    // mutations, leaving the config cell disagreeing with the actually-bound
    // listener. Held for the whole function body; `on_config_change` is the
    // only acquirer, so this never nests.
    let _guard = apply_lock.lock().await;

    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    // Capture the address BEFORE any swap so we can tell a same-address change
    // (rebuild layers live) from a host/port change (rebind the listener).
    let old_addr = {
        let current = cell.read().await;
        format!("{}:{}", current.host, current.port)
    };
    let new_addr = format!("{}:{}", cfg.host, cfg.port);

    if old_addr == new_addr {
        // Same address: swap the snapshot, then rebuild the
        // CORS/timeout/concurrency layers into the live router. Listener stays.
        if apply_config(cell, cfg).await {
            let snapshot = cell.read().await.clone();
            server::rebuild_layers(&hot_router.inner, &snapshot).await;
            tracing::info!("http configuration reloaded (same address)");
        }
        return;
    }

    // Address change: rebind the listener (bind-new-before-stop-old).
    match rebind(cell, hot_router, control, cfg).await {
        Ok(()) => tracing::info!(
            old = %old_addr,
            new = %new_addr,
            "http server rebound after configuration change; old address shutting down"
        ),
        Err(e) => tracing::error!(
            error = %e,
            old = %old_addr,
            new = %new_addr,
            "http rebind failed; keeping previous config and server"
        ),
    }
}

/// Rebind the listener to `cfg`'s new host/port. Resolves every fallible
/// prerequisite BEFORE mutating live state: the config is validated and the NEW
/// address is bound first, so a rejected config or a failed bind leaves the old
/// config AND old server untouched. Once the new bind succeeds: swap the config
/// cell, rebuild the router layers on the SHARED cell (the new server serves via
/// it), spawn the new server, then gracefully shut the old one down (with a hard
/// abort as a safety net). Mirrors the engine's `apply_config` address branch.
async fn rebind(
    cell: &ConfigCell,
    hot_router: &HotRouter,
    control: &ServerControlCell,
    cfg: RestApiConfig,
) -> anyhow::Result<()> {
    if let Err(reason) = cfg.validate() {
        anyhow::bail!("rejected new config: {reason}");
    }

    let new_addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = TcpListener::bind(&new_addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind {new_addr}: {e}"))?;

    // New bind succeeded — now safe to mutate live state. Swap the config and
    // rebuild the router layers on the shared cell BEFORE spawning, so the new
    // server serves the current routes/layers from the first request.
    *cell.write().await = Arc::new(cfg);
    let snapshot = cell.read().await.clone();
    server::rebuild_layers(&hot_router.inner, &snapshot).await;

    let new_control = server::spawn_server(listener, hot_router.clone());

    // Install the new server as current; gracefully drain the old one.
    let old = control.lock().await.replace(new_control);
    if let Some(old) = old {
        server::stop_old_server(old);
    }
    Ok(())
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
