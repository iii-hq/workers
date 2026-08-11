//! Integration with the `configuration` worker (docs/sops/configuration.md,
//! Tier 1) — register the JSON Schema + optional seed at boot, read the
//! authoritative (env-expanded) value, and bind a `configuration` trigger so
//! `configuration:updated` re-fetches and swaps the snapshot. Mirrors
//! [`browser`](../../browser/src/configuration.rs).
//!
//! The output caps and timeout knobs are read per call and hot-apply on the
//! swap; the engine-structural fields are boot-captured, so the reload
//! handler warns when a change to them will only apply at the next restart
//! (see `CodeRunnerConfig::restart_required`).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{CodeRunnerConfig, SharedConfig};

pub const CONFIG_ID: &str = "code-runner";
/// Internal hot-reload hook; denied to agents in iii-permissions.yaml and
/// seeded into the runtime-id registry (functions::seeded_ids) so a guest
/// `register_function` cannot claim it.
pub const CONFIG_FN_ID: &str = "code-runner::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

pub async fn register_config(
    iii: &IIIClient,
    seed: Option<&CodeRunnerConfig>,
) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "code-runner",
        "description": "Runtime limits for in-process Node.js/Python: output caps and timeouts \
                        (hot-reload), plus runtime-count, memory, and scratch limits (applied \
                        at worker restart).",
        "schema": CodeRunnerConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default(iii).await? {
        payload["initial_value"] = CodeRunnerConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<CodeRunnerConfig, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => CodeRunnerConfig::from_json(&v),
        _ => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(CodeRunnerConfig::default())
        }
    }
}

async fn should_seed_default(iii: &IIIClient) -> Result<bool, String> {
    match try_get_value(iii).await? {
        None => Ok(true),
        Some(v) if v.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

/// `Ok(None)` when the entry does not exist yet. The engine's missing-entry
/// codes vary in case, so match case-insensitively.
async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeRequest {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeResponse {
    ok: bool,
}

/// Register the internal reload hook and bind the `configuration` trigger.
/// Call LAST in boot, after every function and the console UI exist.
pub fn register_config_trigger(iii: &IIIClient, config: SharedConfig) -> Result<(), Error> {
    let cfg = config.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_req: OnConfigChangeRequest| {
            let cfg = cfg.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cfg).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload code-runner settings from the authoritative configuration on \
             change. Output caps and timeouts hot-apply; runtime/memory/scratch limits apply \
             at the next worker restart.",
        )
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({ "configuration_id": CONFIG_ID, "event_types": ["configuration:updated"] }),
        metadata: None,
    })?;
    Ok(())
}

/// Re-fetch the AUTHORITATIVE value and swap the snapshot. The trigger
/// payload is deliberately ignored: `on-config-change` is a discoverable bus
/// function, and trusting a caller-supplied value would let anyone inject
/// config without updating persisted state.
async fn on_config_change(iii: &IIIClient, config: &SharedConfig) {
    match fetch_config(iii).await {
        Ok(next) => {
            if config.load().restart_required(&next) {
                tracing::warn!(
                    "configuration changed a boot-captured field (runtimes/memory/scratch); \
                     the change is saved but applies at the next worker restart"
                );
            }
            config.store(Arc::new(next));
            tracing::info!("code-runner configuration reloaded");
        }
        Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
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

    /// The store()/load() path the reload handler uses: a swap must be
    /// visible to the next per-call read.
    #[test]
    fn snapshot_swap_is_visible_to_readers() {
        let shared = CodeRunnerConfig::default().into_shared();
        assert_eq!(shared.load().max_result_bytes, 32_768);
        shared.store(Arc::new(CodeRunnerConfig {
            max_result_bytes: 7,
            ..CodeRunnerConfig::default()
        }));
        assert_eq!(shared.load().max_result_bytes, 7);
    }
}
