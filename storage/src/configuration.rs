//! Integration with the `configuration` worker — register, fetch, and hot-reload
//! the `storage` configuration entry.

use crate::backend::factory::{self, LocalBackendCtx};
use crate::backend::Backend;
use crate::config::{Topology, WorkerConfig};
use crate::handlers::AppState;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_CONFIG_ID: &str = "storage";

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
const CONFIG_FN_ID: &str = "storage::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register the `storage` configuration schema with the configuration worker.
/// When `seed` is present, its value is installed as `initial_value`. Otherwise,
/// the built-in default is seeded only when no stored value exists yet.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Storage",
        "description": "Object storage buckets across AWS S3, GCS, Cloudflare R2, and a managed local rustfs backend.",
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

/// Read the live `storage` configuration (env-expanded by the configuration worker).
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
        .ok_or_else(|| format!("configuration `{config_entry}` not found", config_entry = config_id()))
}

/// Returns `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": config_id() })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Build object-storage backends for every configured bucket. `local_ctx` is the
/// running rustfs sidecar context (port + ephemeral credentials); it must be
/// `Some` whenever the config declares a `provider: local` bucket, otherwise the
/// local backend build fails. On hot-reload the boot-time `local_ctx` is reused —
/// the sidecar is never respawned.
pub async fn build_backends(
    cfg: &WorkerConfig,
    local_ctx: Option<&LocalBackendCtx>,
) -> Result<HashMap<String, Arc<dyn Backend>>, String> {
    let mut backends = HashMap::new();
    for (name, bucket_cfg) in &cfg.buckets {
        let b = factory::build(name, bucket_cfg, &cfg.providers, local_ctx)
            .await
            .map_err(|e| format!("building backend `{name}`: {e}"))?;
        tracing::info!(bucket = %name, provider = b.provider(), "backend ready");
        backends.insert(name.clone(), b);
    }
    Ok(backends)
}

/// Decide whether a freshly-fetched config can be hot-applied. Returns the
/// config when its topology matches the boot-time topology (only
/// backend-connection settings changed), or an error describing the
/// topology change that requires a worker restart.
fn reloadable(cfg: WorkerConfig, boot_topology: &Topology) -> Result<WorkerConfig, String> {
    if cfg.topology() != *boot_topology {
        return Err(
            "configuration change alters bucket/notification topology (bucket add/remove, \
             provider, underlying-bucket, notification-source, or local data-dir)"
                .to_string(),
        );
    }
    Ok(cfg)
}

/// Rebuild the backend map from `cfg` and hot-swap it under the write lock. The
/// rustfs sidecar, webhook receiver, and notification pollers are NOT touched —
/// adding/removing a local bucket or changing a notifications source requires a
/// worker restart. A failed rebuild leaves the running backends untouched.
pub async fn apply_config(state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
    let new_backends = build_backends(&cfg, state.local_ctx.as_ref()).await?;
    let mut backends_guard = state.backends.write().await;
    *backends_guard = new_backends;
    Ok(())
}

/// Internal `storage::on-config-change` trigger payload. The handler re-fetches
/// the authoritative configuration, so this carries only the (advisory)
/// configuration id; a struct (not `Value`) keeps the request schema concrete
/// and unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `storage::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration` trigger.
///
/// `boot_topology` is the bucket/notification topology captured at startup; any
/// reload that would change it is refused (those require a worker restart).
pub fn register_config_trigger(
    iii: &IIIClient,
    state: AppState,
    boot_topology: Topology,
) -> Result<(), Error> {
    let st = state.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let st = st.clone();
            let engine = engine.clone();
            let boot_topology = boot_topology.clone();
            async move {
                on_config_change(&engine, &st, &boot_topology).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: rebuild storage backends from the authoritative configuration when it changes.",
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

/// Reload backends from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `storage::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller inject arbitrary backend config
/// (redirecting writes or wiping backends) without updating persisted state.
/// Instead we re-fetch the stored value via `configuration::get`. A
/// topology-changing update is refused (it requires a restart).
async fn on_config_change(iii: &IIIClient, state: &AppState, boot_topology: &Topology) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous backends"
            );
            return;
        }
    };
    let cfg = match reloadable(cfg, boot_topology) {
        Ok(cfg) => cfg,
        Err(reason) => {
            tracing::warn!(
                reason = %reason,
                "config-change refused: topology change requires a worker restart; keeping previous backends"
            );
            return;
        }
    };
    match apply_config(state, cfg).await {
        Ok(()) => tracing::info!(
            "storage backends reloaded after configuration change (topology unchanged; only backend connection settings updated)"
        ),
        Err(e) => tracing::error!(
            error = %e,
            "failed to rebuild backends after configuration change; keeping previous backends"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;

    fn cfg(yaml: &str) -> WorkerConfig {
        WorkerConfig::from_yaml(yaml).unwrap()
    }

    #[test]
    fn reloadable_allows_credential_only_change() {
        let boot = cfg("buckets:\n  up:\n    provider: r2\n    account_id: a\n    access_key_id: k1\n    secret_access_key: s1\n");
        let next = cfg("buckets:\n  up:\n    provider: r2\n    account_id: a\n    access_key_id: k2\n    secret_access_key: s2\n");
        assert!(reloadable(next, &boot.topology()).is_ok());
    }

    #[test]
    fn reloadable_refuses_topology_change() {
        let boot = cfg("buckets:\n  up:\n    provider: s3\n    region: us-east-1\n");
        let next = cfg("buckets:\n  up:\n    provider: s3\n    region: us-east-1\n  added:\n    provider: s3\n    region: us-east-1\n");
        assert!(reloadable(next, &boot.topology()).is_err());
    }
}
