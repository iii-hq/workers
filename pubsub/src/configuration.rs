//! Integration with the builtin `configuration` worker.
//!
//! The worker registers its config schema under the id `pubsub` (what the
//! console renders as an editable form), reads the authoritative value before
//! binding, and hot-applies `configuration:updated` events by rebuilding the
//! backend adapter and rebinding the live subscriptions onto it.
//!
//! [`on_config_change`] is serialized end-to-end by an [`ApplyLock`]: the SDK
//! dispatches each function invocation via `tokio::spawn`, so overlapping
//! `configuration:updated` events could otherwise interleave their fetch →
//! decide → swap sequences. Mirrors the builtin's subscriptions/apply lock
//! (engine/src/workers/pubsub/pubsub.rs:508-513).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::adapters::{self, Invoker};
use crate::boot::{ApplyLock, ConfigCell};
use crate::config::PubSubConfig;
use crate::hub::Hub;

pub const CONFIG_ID: &str = "pubsub";
const CONFIG_FN_ID: &str = "pubsub::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
/// Upper bound on every `configuration::*` bus call (mirrors the builtin's
/// `CONFIG_BUS_TIMEOUT` of 10s, configuration.rs:39): a hung provider must not
/// wedge boot or reload.
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

/// Register the `pubsub` configuration entry: schema + metadata refresh on
/// every boot; `initial_value` (the `--config` seed, or built-in defaults) is
/// included only when nothing is stored yet, so runtime edits survive restarts.
pub async fn register_config(iii: &IIIClient, seed: Option<&PubSubConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "PubSub",
        "description": "PubSub worker settings — the pub/sub backend adapter (`local` in-process broadcast, or `redis` for cross-instance delivery). The adapter hot-swaps at runtime: a change rebuilds the backend and re-subscribes live subscriptions onto it.",
        "schema": PubSubConfig::json_schema(),
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
pub async fn fetch_config(iii: &IIIClient) -> Result<PubSubConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => PubSubConfig::from_json(&value),
        _ => {
            tracing::info!("no `{CONFIG_ID}` configuration value stored; using built-in default");
            Ok(PubSubConfig::default())
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

/// Builtin parity (pubsub.rs:545): the effective `(name, config)` pair is the
/// change signal — same pair, no rebuild.
fn swap_needed(current: &PubSubConfig, next: &PubSubConfig) -> bool {
    current.effective_adapter() != next.effective_adapter()
}

/// Register `pubsub::on-config-change` and subscribe it to
/// `configuration:updated` events for the `pubsub` entry. The handler ignores
/// the trigger payload and re-fetches the authoritative value (the bus function
/// is discoverable, so a caller-supplied payload must not be trusted).
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    hub: Arc<Hub>,
    invoker: Arc<dyn Invoker>,
    cell: ConfigCell,
    apply_lock: ApplyLock,
) -> Result<(), Error> {
    let hub_for_fn = hub.clone();
    let invoker_for_fn = invoker.clone();
    let cell_for_fn = cell.clone();
    let apply_lock_for_fn = apply_lock.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let hub = hub_for_fn.clone();
            let invoker = invoker_for_fn.clone();
            let cell = cell_for_fn.clone();
            let apply_lock = apply_lock_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &hub, &invoker, &cell, &apply_lock).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload pubsub configuration from the authoritative store on change."),
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

/// Re-fetch the authoritative config and, if the effective adapter changed,
/// build a new backend and hot-swap it (gated: a build failure keeps both the
/// old backend AND old config — builtin parity, pubsub.rs:551-552). The
/// [`ApplyLock`] serializes overlapping runs (see module docs).
async fn on_config_change(
    iii: &IIIClient,
    hub: &Arc<Hub>,
    invoker: &Arc<dyn Invoker>,
    cell: &ConfigCell,
    apply_lock: &ApplyLock,
) {
    let _guard = apply_lock.lock().await;

    // Never trust the trigger payload; re-fetch the authoritative value
    // (builtin parity, configuration.rs:122-127).
    let next = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "pubsub config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let current = cell.read().await.clone();
    if !swap_needed(&current, &next) {
        *cell.write().await = next;
        tracing::debug!("pubsub configuration reloaded (adapter unchanged)");
        return;
    }

    let new_adapter = match adapters::build_adapter(&next, invoker.clone()).await {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "pubsub: failed to build new adapter; keeping previous backend and config");
            return;
        }
    };

    hub.swap_adapter(new_adapter).await;
    *cell.write().await = next;
    tracing::info!("pubsub configuration reloaded (adapter hot-swapped)");
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

    #[test]
    fn swap_needed_only_when_effective_adapter_changes() {
        let none = PubSubConfig::default();
        let local: PubSubConfig = serde_yaml::from_str("{adapter: {name: local}}").unwrap();
        let redis: PubSubConfig = serde_yaml::from_str("{adapter: {name: redis}}").unwrap();
        let redis_url: PubSubConfig =
            serde_yaml::from_str("{adapter: {name: redis, config: {redis_url: 'redis://y'}}}")
                .unwrap();

        assert!(!swap_needed(&local, &local));
        assert!(!swap_needed(&none, &local), "None normalizes to local (builtin parity)");
        assert!(swap_needed(&local, &redis));
        assert!(swap_needed(&redis, &redis_url), "adapter config change forces a rebuild");
    }
}
