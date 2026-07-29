//! Integration with the `configuration` worker — register the schema, fetch the
//! authoritative value at boot, and hot-reload it when it changes. Mirrors
//! [`context-manager`](../../context-manager/src/configuration.rs).
//!
//! Every field here is a tuning knob: caps, limits and a timeout. None of them
//! is structural — there is no adapter to rebuild and no trigger to re-bind,
//! because this worker owns no resources of its own (files go through `shell`,
//! the workspace record through `state`). So a reload is a snapshot swap, and
//! every handler reads the live snapshot per call rather than capturing one at
//! registration. Nothing requires a restart.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a `read().await`
/// and clone the inner `Arc` out (a refcount bump) without holding the lock
/// across its work, while [`apply_config`] replaces the inner `Arc` under the
/// write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "editor";
const CONFIG_FN_ID: &str = "editor::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the attempt
/// number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

pub fn cell(cfg: WorkerConfig) -> ConfigCell {
    Arc::new(RwLock::new(Arc::new(cfg)))
}

/// Register the `editor` configuration schema.
///
/// When `seed` is present its value is installed as `initial_value`; otherwise
/// the built-in default is seeded only when nothing is stored yet, so calling
/// this on every boot never overwrites an operator's edit.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Editor",
        "description": "Editor workspace limits: diff size and context, file-finder and \
                        content-search caps, the largest file an open will pull back, and \
                        the per-git-invocation timeout. Nothing here grants access — the \
                        filesystem boundary is the shell worker's jail.",
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

/// Read the live configuration (env-expanded by the configuration worker —
/// `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))?;
    if value.is_null() {
        tracing::info!("no stored configuration; using built-in defaults");
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

/// `Ok(None)` when the entry does not exist. The engine's missing-entry codes
/// vary in case (`function_not_found`, `NOT_FOUND`), so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last = String::new();
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
            Ok(value) => return Ok(value),
            Err(e) => {
                last = e.to_string();
                // A missing entry is an answer, not a transient failure — do
                // not spend the whole retry budget on it.
                if last.to_ascii_uppercase().contains("NOT_FOUND") {
                    break;
                }
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * attempt as u64,
                    ))
                    .await;
                }
            }
        }
    }
    Err(last)
}

/// Swap the snapshot. Handlers read it per call, so the next invocation of
/// every function sees the new values.
pub async fn apply_config(cell: &ConfigCell, git_timeout_ms: &AtomicU64, cfg: WorkerConfig) {
    // The bus holds the git timeout separately (it is read on a path that has
    // no access to the snapshot), so it has to be published here or it would
    // be the one field a reload silently skipped.
    git_timeout_ms.store(cfg.git_timeout_ms, Ordering::Relaxed);
    *cell.write().await = Arc::new(cfg);
}

/// Internal `editor::on-config-change` payload. The handler re-fetches the
/// authoritative value, so this carries only the (advisory) id; a struct rather
/// than a `Value` keeps the request schema concrete.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. Registered here rather than in `functions::register_all` so it
/// stays off the public `catalog()`.
pub fn register_config_trigger(
    iii: &IIIClient,
    cell: ConfigCell,
    git_timeout_ms: Arc<AtomicU64>,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let timeout_for_fn = git_timeout_ms.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let timeout = timeout_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &timeout).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the editor's limits from the authoritative configuration \
             when it changes.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
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
/// The trigger payload is deliberately ignored: `editor::on-config-change` is a
/// bus function, so trusting a caller-supplied value would let anyone inject
/// limits without updating persisted state. A failed fetch keeps the previous
/// snapshot (last-good) rather than falling back to defaults, which would
/// silently widen every cap.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, git_timeout_ms: &AtomicU64) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(cell, git_timeout_ms, cfg).await;
            tracing::info!("editor configuration reloaded");
        }
        Err(e) => tracing::error!(
            error = %e,
            "config-change: failed to fetch authoritative configuration; keeping previous config"
        ),
    }
}
