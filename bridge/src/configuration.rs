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
//! - An **unchanged `url`** registers only the `expose` entries never before
//!   registered on the current remote client.
//! - `forward` entries never before registered on the local client get their
//!   local proxy function registered; existing ones keep their handler (it
//!   reads the [`crate::functions::ForwardTable`] per call, so an edited
//!   `remote_function`/`timeout_ms` takes effect once the table is replaced).
//! - Both tables are replaced wholesale with the new config's entries BEFORE
//!   any registration, so a just-added entry finds its table row the instant
//!   its function becomes routable. The SDK has no unregister, so a REMOVED
//!   entry is not un-registered — its handler keeps running but the table
//!   lookup fails, and the handler returns a `bridge_error` (see
//!   `functions::handle_forward` / `functions::handle_expose`).
//!
//! ## Ever-registered id sets (remove -> re-add safety)
//!
//! The SDK PANICS on a duplicate `register_function` id and has no
//! unregister, so "does this entry still need registering?" must be answered
//! against the ids EVER registered on a client — not against the current
//! tables. Otherwise an entry removed in one update and re-added in the next
//! would look "new", get re-registered, and panic the reload task.
//! [`crate::boot::BootHandle::local_registered`] tracks every id registered
//! on the local client (seeded at boot with `bridge.invoke` /
//! `bridge.invoke_async` and the boot forwards);
//! [`crate::boot::BootHandle::remote_registered`] tracks the ids registered
//! on the CURRENT remote-client generation and is reset when a `url` change
//! swaps in a fresh client. A re-added entry needs no re-registration: the
//! table swap alone revives it, because handlers do per-call table lookups.
//!
//! [`on_config_change`] is serialized end-to-end by an [`ApplyLock`]: the SDK
//! dispatches each function invocation via `tokio::spawn`, so overlapping
//! `configuration:updated` events could otherwise interleave their
//! [`ConfigCell`] / [`crate::functions::RemoteCell`] / table mutations.
//! Mirrors the engine's `apply_lock` (`api_core.rs`) and `http`'s
//! [`ApplyLock`] (`http/src/configuration.rs`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::boot::{self, BootHandle, RegisteredIds};
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

/// The clonable slice of [`BootHandle`] state a reload run mutates.
#[derive(Clone)]
struct ReloadCtx {
    cell: ConfigCell,
    remote: RemoteCell,
    forwards: ForwardTable,
    exposes: ExposeTable,
    local_registered: RegisteredIds,
    remote_registered: RegisteredIds,
    apply_lock: ApplyLock,
}

impl ReloadCtx {
    fn from_boot(boot: &BootHandle) -> Self {
        Self {
            cell: boot.config.clone(),
            remote: boot.remote.clone(),
            forwards: boot.forwards.clone(),
            exposes: boot.exposes.clone(),
            local_registered: boot.local_registered.clone(),
            remote_registered: boot.remote_registered.clone(),
            apply_lock: boot.apply_lock.clone(),
        }
    }
}

/// Register `bridge::on-config-change` and subscribe it to
/// `configuration:updated` events for the `bridge` entry. The handler ignores
/// the trigger payload and re-fetches the authoritative value (the bus
/// function is discoverable, so a caller-supplied payload must not be trusted
/// to repoint config).
pub fn register_config_trigger(iii: &Arc<IIIClient>, boot: &BootHandle) -> Result<(), Error> {
    let ctx = ReloadCtx::from_boot(boot);
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let ctx = ctx.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &ctx).await;
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

/// Names in `next` never registered on the client — the only entries whose
/// functions still need registering. A previously-registered name (even one
/// removed from the current tables) is NOT returned: the SDK panics on a
/// duplicate `register_function` id, and the table swap alone revives it.
fn added_keys(
    ever_registered: &HashSet<String>,
    next: impl Iterator<Item = String>,
) -> Vec<String> {
    next.filter(|k| !ever_registered.contains(k)).collect()
}

/// Reset a per-generation registered set when a `url` change swaps in a fresh
/// remote client: the new client starts with zero registrations, so the set
/// is reseeded with exactly the names registered on it.
fn reset_generation(ever_registered: &mut HashSet<String>, names: impl Iterator<Item = String>) {
    ever_registered.clear();
    ever_registered.extend(names);
}

async fn on_config_change(iii: &Arc<IIIClient>, ctx: &ReloadCtx) {
    // Serialize the whole re-fetch -> swap-table -> connect/register -> swap-
    // cell sequence: the SDK dispatches each invocation via `tokio::spawn`, so
    // two `configuration:updated` events can run this function concurrently.
    // Without this, overlapping edits could interleave their remote/table/
    // cell mutations. Held for the whole function body; `on_config_change` is
    // the only acquirer, so this never nests.
    let _guard = ctx.apply_lock.lock().await;

    // 1. Fetch the authoritative value; never trust the trigger payload.
    let next = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    // 2. Snapshot the current config and compute env_url once for this run.
    let current = ctx.cell.read().await.clone();
    let env_url = std::env::var("III_URL").ok();
    let url_did_change = url_changed(&current, &next, env_url.clone());

    // 3. Compute additions against the EVER-registered id sets (never the
    // tables): a removed-then-re-added name must not be re-registered — the
    // SDK panics on duplicate ids — and needs none, the table swap below
    // revives its still-running handler.
    let added_forwards = {
        let ever = ctx.local_registered.read().await;
        added_keys(&ever, next.forward.iter().map(|f| f.local_function.clone()))
    };
    let added_exposes = if url_did_change {
        Vec::new() // a fresh client gets EVERY expose entry registered below
    } else {
        let ever = ctx.remote_registered.read().await;
        added_keys(
            &ever,
            next.expose.iter().map(|e| e.remote_name().to_string()),
        )
    };

    // 4. Replace both table contents wholesale FIRST: removed entries start
    // failing their lookup with a bridge_error (the documented SDK-no-
    // unregister limitation), re-added entries revive, and a just-added entry
    // finds its row the instant its function registers (no registered-before-
    // table window).
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
    *ctx.forwards.write().await = new_forwards;
    *ctx.exposes.write().await = new_exposes;

    if url_did_change {
        // 5a. URL changed: connect the new client first (background-
        // connecting, cannot fail synchronously), register EVERY expose entry
        // on it, reseed the per-generation registered set, then swap it into
        // the cell and shut the old client down.
        let new_client = Arc::new(register_worker(
            &next.effective_url_with(env_url),
            InitOptions::default(),
        ));
        for e in &next.expose {
            boot::register_expose_function(
                &new_client,
                iii.clone(),
                ctx.exposes.clone(),
                e.remote_name(),
            );
        }
        reset_generation(
            &mut *ctx.remote_registered.write().await,
            next.expose.iter().map(|e| e.remote_name().to_string()),
        );
        let old_client = {
            let mut guard = ctx.remote.write().await;
            std::mem::replace(&mut *guard, new_client)
        };
        old_client.shutdown_async().await;
        tracing::info!("bridge remote client reconnected after configuration change");
    } else if !added_exposes.is_empty() {
        // 5b. URL unchanged: register only the never-registered expose
        // entries on the current remote client.
        let client = ctx.remote.read().await.clone();
        for e in next
            .expose
            .iter()
            .filter(|e| added_exposes.contains(&e.remote_name().to_string()))
        {
            boot::register_expose_function(
                &client,
                iii.clone(),
                ctx.exposes.clone(),
                e.remote_name(),
            );
        }
        ctx.remote_registered
            .write()
            .await
            .extend(added_exposes.iter().cloned());
    }

    // 6. Forwards: register only the never-registered local proxy functions.
    // Existing ids keep their handler — it reads the table per call, so a
    // changed remote_function/timeout_ms took effect at step 4.
    for f in next
        .forward
        .iter()
        .filter(|f| added_forwards.contains(&f.local_function))
    {
        boot::register_forward_function(
            iii,
            ctx.remote.clone(),
            ctx.forwards.clone(),
            &f.local_function,
            &f.remote_function,
        );
    }
    ctx.local_registered
        .write()
        .await
        .extend(added_forwards.iter().cloned());

    // 7. Publish the new config snapshot last — the observable "reload done".
    *ctx.cell.write().await = Arc::new(next.normalized());
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

    fn cfg(yaml: &str) -> BridgeConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
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
    fn added_keys_returns_only_never_registered_names() {
        let ever = set(&["kept"]);
        let next = vec!["kept".to_string(), "new".to_string()];
        assert_eq!(added_keys(&ever, next.into_iter()), vec!["new".to_string()]);
    }

    /// Remove -> re-add regression (F1): a name EVER registered — even one no
    /// longer in the current tables — must yield NO addition, because the SDK
    /// panics on duplicate registration; the table swap alone revives it.
    #[test]
    fn added_keys_skips_removed_then_readded_name() {
        // v1 registered "fwd"; v2 removed it (tables no longer contain it,
        // but the ever-registered set still does); v3 re-adds it.
        let ever = set(&["fwd", "bridge.invoke", "bridge.invoke_async"]);
        let v3 = vec!["fwd".to_string()];
        assert!(
            added_keys(&ever, v3.into_iter()).is_empty(),
            "re-added name must not be re-registered"
        );
    }

    /// A url swap replaces the remote client, so the per-generation set is
    /// reseeded: names from the old generation are forgotten (the new client
    /// never saw them) and exactly the new registrations are tracked.
    #[test]
    fn reset_generation_reseeds_registered_set_on_url_swap() {
        let mut ever = set(&["old.expose", "kept.expose"]);
        reset_generation(
            &mut ever,
            vec!["kept.expose".to_string(), "new.expose".to_string()].into_iter(),
        );
        assert_eq!(ever, set(&["kept.expose", "new.expose"]));
        // "old.expose" re-added on the NEXT generation counts as new again.
        assert_eq!(
            added_keys(&ever, vec!["old.expose".to_string()].into_iter()),
            vec!["old.expose".to_string()]
        );
    }
}
