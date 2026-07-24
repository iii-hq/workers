//! Integration with the `configuration` worker — register, fetch, and
//! hot-reload the `iii-directory` configuration entry.
//!
//! Mirrors the `database` worker's pattern: register a JSON Schema + seed
//! at boot, read the authoritative (env-expanded) value via
//! `configuration::get`, and bind a `configuration` trigger so a
//! `configuration:updated` event re-fetches and applies the change.
//!
//! Tunable fields (`registry_url`, `download_timeout_ms`,
//! `registry_cache_ttl_ms`, `filter_unregistered`) hot-reload in place;
//! topology fields (`skills_folder`, `local_skills_folder`,
//! `auto_download`) are refused with a "restart required" log because
//! they define on-disk roots and boot-time task wiring.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{SharedConfig, SkillsConfig, Topology};
use crate::functions::registry::RegistryCache;
use crate::functions::skills::RegisteredWorkersCache;

pub const CONFIG_ID: &str = "iii-directory";
const CONFIG_FN_ID: &str = "directory::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Runtime resources a `configuration:updated` reload must reach: the live
/// config snapshot handlers read, the shared cache-TTL cell, the two caches
/// to clear, and the immutable boot-time topology used to detect
/// restart-requiring changes.
#[derive(Clone)]
pub struct SharedState {
    /// Live tunable snapshot read by handlers via `load_full()`.
    pub config: SharedConfig,
    /// Shared TTL (ms) backing both caches; updated on reload.
    pub cache_ttl_ms: Arc<AtomicU64>,
    /// Registry HTTP-response cache; cleared on reload.
    pub registry_cache: RegistryCache,
    /// Installed-worker-name cache; invalidated on reload.
    pub registered_cache: Arc<RegisteredWorkersCache>,
    /// Restart-only fields captured at boot.
    boot_topology: Topology,
}

impl SharedState {
    pub fn new(
        config: SharedConfig,
        cache_ttl_ms: Arc<AtomicU64>,
        registry_cache: RegistryCache,
        registered_cache: Arc<RegisteredWorkersCache>,
        boot_topology: Topology,
    ) -> Self {
        Self {
            config,
            cache_ttl_ms,
            registry_cache,
            registered_cache,
            boot_topology,
        }
    }
}

/// Register the `iii-directory` configuration schema with the configuration
/// worker. When `seed` is present, its value is installed as `initial_value`.
/// Otherwise, built-in defaults are seeded only when no stored value exists.
pub async fn register_config(iii: &IIIClient, seed: Option<&SkillsConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "iii-directory",
        "description": "Skills/prompts folders, workers-registry URL, download timeouts, \
                        and skill-visibility filters for the iii-directory worker.",
        "schema": SkillsConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = SkillsConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `iii-directory` configuration (env-expanded by the
/// configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<SkillsConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(SkillsConfig::default());
    }
    SkillsConfig::from_json(&value)
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

/// Returns `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Apply a freshly-fetched configuration: swap the live snapshot, update the
/// shared cache TTL, and clear both caches so a repointed `registry_url`
/// takes effect immediately and stale entries from the old registry drop.
pub async fn apply_config(state: &SharedState, cfg: SkillsConfig) {
    state
        .cache_ttl_ms
        .store(cfg.registry_cache_ttl_ms, Ordering::Relaxed);
    state.config.store(Arc::new(cfg));
    state.registry_cache.clear().await;
    state.registered_cache.invalidate().await;
}

/// Trigger payload for `directory::on-config-change`. The handler ignores it
/// (it re-fetches from the authoritative configuration); the empty struct
/// exists so the function publishes a typed request schema rather than AnyValue.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeRequest {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeResponse {
    ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger for `configuration:updated` on the `iii-directory` entry.
pub fn register_config_trigger(iii: &IIIClient, state: SharedState) -> Result<(), Error> {
    let st = state.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_req: OnConfigChangeRequest| {
            let st = st.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &st).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload tunable iii-directory settings from the authoritative \
             configuration when it changes.",
        )
        .metadata(json!({ "internal": true })),
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
/// `directory::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller repoint the registry URL or
/// download roots without updating persisted state. Re-fetch the stored
/// value via `configuration::get` instead. Topology changes are refused —
/// the on-disk roots and the auto-download wiring are fixed at boot.
async fn on_config_change(iii: &IIIClient, state: &SharedState) {
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
    if cfg.topology() != state.boot_topology {
        tracing::warn!(
            "configuration change alters topology (skills_folder, local_skills_folder, \
             auto_download, or inject_guidance); a restart is required to apply it — keeping \
             previous configuration"
        );
        return;
    }
    apply_config(state, cfg).await;
    tracing::info!("iii-directory configuration reloaded (tunable fields applied; caches cleared)");
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
