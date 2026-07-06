//! Integration with the builtin `configuration` worker.
//!
//! The worker registers its config schema under the id `bridge` (what the
//! console renders as an editable form), reads the authoritative value before
//! connecting, and hot-applies `configuration:updated` events onto a shared
//! [`ConfigCell`].
//!
//! ## What reloads live vs. restart-only
//!
//! [`on_config_change`] re-fetches the authoritative config (never trusting
//! the trigger payload) and applies it in place:
//!
//! - A **`url`** change connects a NEW remote [`IIIClient`] first (background
//!   connecting — cannot fail synchronously), registers every `expose` entry
//!   on it, swaps it into the shared [`crate::functions::RemoteCell`], then
//!   gracefully shuts the old client down. Connect-new-before-drop-old mirrors
//!   `http`'s bind-new-before-stop-old rebind.
//! - An **unchanged `url`** registers only the newly-added `expose` entries on
//!   the current remote client.
//! - Newly-added `forward` entries get their local proxy function registered;
//!   existing ones keep their handler (it reads the [`crate::functions::ForwardTable`]
//!   per call, so an edited `remote_function`/`timeout_ms` takes effect once
//!   the table is replaced below).
//! - Both tables are then replaced wholesale with the new config's entries.
//!   The SDK has no unregister, so a REMOVED entry is not un-registered — its
//!   handler keeps running but the table lookup fails, and the handler
//!   returns a `bridge_error` (see `functions::handle_forward` /
//!   `functions::handle_expose`).
//!
//! [`on_config_change`] is serialized end-to-end by an [`ApplyLock`]: the SDK
//! dispatches each function invocation via `tokio::spawn`, so overlapping
//! `configuration:updated` events could otherwise interleave their
//! [`ConfigCell`] / [`crate::functions::RemoteCell`] / table mutations.
//! Mirrors the engine's `apply_lock` (`api_core.rs`) and `http`'s
//! [`ApplyLock`] (`http/src/configuration.rs`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::boot;
use crate::config::BridgeConfig;
use crate::functions::{ExposeTable, ForwardTable, RemoteCell};

/// The live config cell, swappable on a config update.
pub type ConfigCell = Arc<RwLock<Arc<BridgeConfig>>>;

/// Apply lock to serialize overlapping configuration change runs.
pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;

pub const CONFIG_ID: &str = "bridge";
const CONFIG_FN_ID: &str = "bridge::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
/// Upper bound on every `configuration::*` bus call (mirrors the engine's
/// `CONFIG_BUS_TIMEOUT` of 10s): a hung provider must not wedge boot or reload.
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

/// Register the `bridge` configuration entry: schema + metadata refresh on
/// every boot; `initial_value` (the `--config` seed, or built-in defaults) is
/// included only when nothing is stored yet, so runtime edits survive
/// restarts.
pub async fn register_config(iii: &IIIClient, seed: Option<&BridgeConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Bridge",
        "description": "Bridge to a remote iii instance — remote WebSocket URL, functions exposed to the remote engine, and remote functions forwarded locally.",
        "schema": BridgeConfig::json_schema(),
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
/// built-in default; a malformed stored value is an error so callers keep
/// their previous config.
pub async fn fetch_config(iii: &IIIClient) -> Result<BridgeConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => BridgeConfig::from_json(&value),
        _ => {
            tracing::info!("no `{CONFIG_ID}` configuration value stored; using built-in default");
            Ok(BridgeConfig::default())
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

/// Register `bridge::on-config-change` and subscribe it to
/// `configuration:updated` events for the `bridge` entry. The handler ignores
/// the trigger payload and re-fetches the authoritative value (the bus
/// function is discoverable, so a caller-supplied payload must not be trusted
/// to repoint config).
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    remote: RemoteCell,
    forwards: ForwardTable,
    exposes: ExposeTable,
    apply_lock: ApplyLock,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let remote_for_fn = remote.clone();
    let forwards_for_fn = forwards.clone();
    let exposes_for_fn = exposes.clone();
    let apply_lock_for_fn = apply_lock.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let cell = cell_for_fn.clone();
            let remote = remote_for_fn.clone();
            let forwards = forwards_for_fn.clone();
            let exposes = exposes_for_fn.clone();
            let apply_lock = apply_lock_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &remote, &forwards, &exposes, &apply_lock).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description(
            "Internal: reload bridge configuration from the authoritative store on change.",
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

/// True when the effective remote URL (config value, falling back to
/// `env_url`, falling back to the built-in default) differs between `current`
/// and `next`.
fn url_changed(current: &BridgeConfig, next: &BridgeConfig, env_url: Option<String>) -> bool {
    current.effective_url_with(env_url.clone()) != next.effective_url_with(env_url)
}

/// Names present in `next` but not in `old` — the entries whose functions
/// still need registering (the SDK has no unregister, so removals are handled
/// by the table lookup failing in the handler instead).
fn added_keys<V>(old: &HashMap<String, V>, next: impl Iterator<Item = String>) -> Vec<String> {
    next.filter(|k| !old.contains_key(k)).collect()
}

async fn on_config_change(
    iii: &Arc<IIIClient>,
    cell: &ConfigCell,
    remote: &RemoteCell,
    forwards: &ForwardTable,
    exposes: &ExposeTable,
    apply_lock: &ApplyLock,
) {
    // Serialize the whole re-fetch -> connect/register -> swap-cell/table
    // sequence: the SDK dispatches each invocation via `tokio::spawn`, so two
    // `configuration:updated` events can run this function concurrently.
    // Without this, overlapping edits could interleave their remote/table/
    // cell mutations. Held for the whole function body; `on_config_change` is
    // the only acquirer, so this never nests.
    let _guard = apply_lock.lock().await;

    // 1. Fetch the authoritative value; never trust the trigger payload.
    let next = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    // 2. Snapshot the current config and compute env_url once for this run.
    let current = cell.read().await.clone();
    let env_url = std::env::var("III_URL").ok();

    if url_changed(&current, &next, env_url) {
        // 3. URL changed: connect the new client first (background-connecting,
        // cannot fail synchronously), register EVERY expose entry on it, then
        // swap it into the cell and shut the old client down.
        let new_client = Arc::new(register_worker(
            &next.effective_url(),
            InitOptions::default(),
        ));
        for e in &next.expose {
            boot::register_expose_function(
                &new_client,
                iii.clone(),
                exposes.clone(),
                e.remote_name(),
            );
        }
        let old_client = {
            let mut guard = remote.write().await;
            std::mem::replace(&mut *guard, new_client)
        };
        old_client.shutdown_async().await;
        tracing::info!("bridge remote client reconnected after configuration change");
    } else {
        // 4. URL unchanged: register only the newly-added expose entries on
        // the current remote client.
        let added = {
            let exposes_read = exposes.read().await;
            added_keys(
                &exposes_read,
                next.expose.iter().map(|e| e.remote_name().to_string()),
            )
        };
        if !added.is_empty() {
            let client = remote.read().await.clone();
            for e in next
                .expose
                .iter()
                .filter(|e| added.contains(&e.remote_name().to_string()))
            {
                boot::register_expose_function(
                    &client,
                    iii.clone(),
                    exposes.clone(),
                    e.remote_name(),
                );
            }
        }
    }

    // 5. Forwards: register only the newly-added local proxy functions.
    // Existing ids keep their handler — it reads the table per call, so a
    // changed remote_function/timeout_ms takes effect via step 6.
    let added_forwards = {
        let forwards_read = forwards.read().await;
        added_keys(
            &forwards_read,
            next.forward.iter().map(|f| f.local_function.clone()),
        )
    };
    for f in next
        .forward
        .iter()
        .filter(|f| added_forwards.contains(&f.local_function))
    {
        boot::register_forward_function(
            iii,
            remote.clone(),
            forwards.clone(),
            &f.local_function,
            &f.remote_function,
        );
    }

    // 6. Replace both table contents wholesale (removed entries now fail
    // their lookup with a bridge_error — the documented SDK-no-unregister
    // limitation), then publish the new config snapshot.
    let new_forwards: HashMap<String, crate::config::ForwardEntry> = next
        .forward
        .iter()
        .map(|f| (f.local_function.clone(), f.clone()))
        .collect();
    let new_exposes: HashMap<String, crate::config::ExposeEntry> = next
        .expose
        .iter()
        .map(|e| (e.remote_name().to_string(), e.clone()))
        .collect();
    *forwards.write().await = new_forwards;
    *exposes.write().await = new_exposes;
    *cell.write().await = Arc::new(next.normalized());
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
    use std::collections::HashMap;

    fn cfg(yaml: &str) -> BridgeConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn url_change_detected_through_fallback_chain() {
        let a = cfg("{url: 'ws://a:1'}");
        let b = cfg("{url: 'ws://b:2'}");
        let none = BridgeConfig::default();
        assert!(url_changed(&a, &b, None));
        assert!(!url_changed(&a, &a, None));
        // explicit url vs fallback-to-env
        assert!(url_changed(&a, &none, Some("ws://env:9".into())));
        // both fall back to the same env value -> no change
        assert!(!url_changed(&none, &none, Some("ws://env:9".into())));
    }

    #[test]
    fn added_keys_returns_only_new_names() {
        let mut old = HashMap::new();
        old.insert("kept".to_string(), ());
        let next = vec!["kept".to_string(), "new".to_string()];
        assert_eq!(added_keys(&old, next.into_iter()), vec!["new".to_string()]);
    }
}
