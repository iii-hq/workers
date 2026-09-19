//! Integration with the `configuration` dependency worker for runtime config values.
//!
//! The worker registers its config schema under the id `state` (validated
//! when its explicit global Settings form saves), reads the authoritative
//! value before binding, and hot-applies `configuration:updated` events onto the shared
//! [`crate::functions::ConfigCell`].
//!
//! ## Three apply tiers (builtin parity, `state.rs:337-402`)
//!
//! - **LIVE** — swapping the config cell alone applies `triggers_enabled` and
//!   `max_value_bytes`: the `state::*` function handlers read a fresh
//!   snapshot per call (see `StateCtx::snapshot`).
//! - **TASK-REBUILD** — a `save_interval_ms` change respawns the adapter's
//!   save loop via [`crate::adapters::StateAdapter::reconfigure`].
//! - **RESTART-ONLY** — an `adapter` change is logged and only takes effect
//!   at the next worker start (the persisted entry is read at boot).
//!
//! `on_config_change` is serialized end-to-end by an `ApplyLock`: the SDK
//! dispatches each function invocation via `tokio::spawn`, so overlapping
//! `configuration:updated` events could otherwise interleave their
//! [`crate::functions::ConfigCell`] mutations. Mirrors the engine's
//! `apply_lock` (`api_core.rs`).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{Value, json};

use crate::boot::ApplyLock;
use crate::config::StateConfig;
use crate::functions::StateCtx;

pub const DEFAULT_CONFIG_ID: &str = "state";

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
const CONFIG_FN_ID: &str = "state::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
/// Upper bound on every `configuration::*` bus call (mirrors the engine's
/// `CONFIG_BUS_TIMEOUT` of 10s): a hung provider must not wedge boot or reload.
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

/// Register the `state` configuration entry: schema + metadata refresh on
/// every boot; `initial_value` (the `--config` seed, or built-in defaults) is
/// included only when nothing is stored yet, so runtime edits survive
/// restarts.
pub async fn register_config(iii: &IIIClient, seed: Option<&StateConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "State",
        "description": "State store settings — storage adapter (kv/redis), trigger fan-out gate, max value size, and file-store flush cadence.",
        "schema": StateConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override
    // (even `false`/`0`/`""`) is preserved without a client-side
    // read-then-register race.
    let seed = seed.cloned().unwrap_or_default().normalized();
    payload["initial_value"] = seed.to_json();
    match trigger_configuration_with_retry(
        iii,
        "configuration::ensure",
        payload,
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(e) if is_function_not_found(&e) => Err(ENSURE_UNAVAILABLE.to_string()),
        Err(e) => Err(e),
    }
}

/// Read the live configuration value. A missing/null value falls back to the
/// built-in default; a malformed stored value is an error so callers keep
/// their previous config.
pub async fn fetch_config(iii: &IIIClient) -> Result<StateConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => StateConfig::from_json(&value),
        _ => {
            tracing::info!(
                "no `{config_entry}` configuration value stored; using built-in default",
                config_entry = config_id()
            );
            Ok(StateConfig::default().normalized())
        }
    }
}

/// Upgrade-required error surfaced when the engine lacks atomic
/// `configuration::ensure` (fail CLOSED; never a legacy seed-over-stored write).
const ENSURE_UNAVAILABLE: &str = "configuration::ensure unavailable; upgrade engine with atomic configuration initialization support";

/// `true` when the error is the engine's lowercase missing-FUNCTION envelope
/// `function_not_found` (an engine without `configuration::ensure`). Same
/// envelope discipline as `is_not_found`: peel the one retry wrapper, then
/// require the envelope at the very start so a stray token still propagates.
fn is_function_not_found(error: &str) -> bool {
    const RETRY_WRAPPER: &str = "configuration::ensure failed after 3 attempts: ";
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "function_not_found"
        || raw == "remote error (function_not_found):"
        || raw.starts_with("remote error (function_not_found): ")
}

/// `true` for the one error that is a definitive answer rather than a
/// failure: the configuration worker's uppercase `NOT_FOUND` entry code.
/// Matched case-SENSITIVELY — the engine's missing-function code is the
/// lowercase `function_not_found` and a backend lookup failure is
/// `statement_not_found`; those must propagate as errors instead of being
/// read as "nothing stored yet", which would seed a default over a stored
/// operator/override value.
fn is_not_found(error: &str) -> bool {
    // Anchor on the SDK's own rendering instead of scanning the whole string:
    // a remote failure prints as `remote error ({code}): {message}`, and this
    // worker wraps a retried get as
    // `configuration::get failed after CONFIG_RETRIES attempts: {err}`. Peel
    // exactly that one wrapper (never a foreign one or a different attempt
    // count) and then require the NOT_FOUND envelope at the very start, so a
    // NOT_FOUND code buried in an unrelated message, a nested envelope, or a
    // different wrapper stays a real failure and propagates.
    const RETRY_WRAPPER: &str = "configuration::get failed after 3 attempts: ";
    const _: () = assert!(CONFIG_RETRIES == 3);
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "NOT_FOUND"
        || raw == "remote error (NOT_FOUND):"
        || raw.starts_with("remote error (NOT_FOUND): ")
}

/// Resolve the authoritative entry without conflating lookup failure with first boot.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(
        iii,
        "configuration::get",
        json!({ "id": config_id() }),
        CONFIG_BUS_TIMEOUT_MS,
    )
    .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// TASK-REBUILD tier: a `save_interval_ms` change respawns the adapter's save
/// loop but doesn't need a full restart.
fn cadence_changed(current: &StateConfig, next: &StateConfig) -> bool {
    current.save_interval_ms != next.save_interval_ms
}

/// RESTART-ONLY tier: the whole `adapter` entry (name and/or inner config) is
/// compared, so an adapter-inner config edit is restart-tier too — builtin
/// parity (`old.adapter != new.adapter` compares the whole entry).
fn adapter_changed(current: &StateConfig, next: &StateConfig) -> bool {
    current.adapter != next.adapter
}

/// Register `state::on-config-change` and subscribe it to
/// `configuration:updated` events for the `state` entry. The handler ignores
/// the trigger payload and re-fetches the authoritative value (the bus
/// function is discoverable, so a caller-supplied payload must not be trusted
/// to repoint config).
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    ctx: Arc<StateCtx>,
    apply_lock: ApplyLock,
) -> Result<(), Error> {
    let ctx_for_fn = ctx.clone();
    let apply_lock_for_fn = apply_lock.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let ctx = ctx_for_fn.clone();
            let apply_lock = apply_lock_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &ctx, &apply_lock).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload state configuration from the authoritative store on change.")
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
    ))?;
    Ok(())
}

async fn on_config_change(iii: &IIIClient, ctx: &Arc<StateCtx>, apply_lock: &ApplyLock) {
    // Serialize the whole re-fetch -> swap-cell -> reconfigure sequence: the
    // SDK dispatches each invocation via `tokio::spawn`, so two
    // `configuration:updated` events can run this function concurrently.
    // Held for the whole function body; `on_config_change` is the only
    // acquirer, so this never nests.
    let _guard = apply_lock.lock().await;

    // Never trust the trigger payload — re-fetch the authoritative value. A
    // fetch failure keeps the previous config in place.
    let new = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous configuration");
            return;
        }
    };

    let old = ctx.config.read().await.clone();

    // LIVE: swap the cell. `triggers_enabled` / `max_value_bytes` are read
    // per call by the function handlers, so this alone applies them.
    *ctx.config.write().await = Arc::new(new.clone());

    // TASK-REBUILD: respawn the adapter's save loop on a cadence change.
    if cadence_changed(&old, &new)
        && let Err(e) = ctx
            .adapter
            .reconfigure(&json!({ "save_interval_ms": new.save_interval_ms }))
            .await
    {
        tracing::warn!(
            error = %e,
            "state: save_interval_ms reconfigure failed; cadence unchanged"
        );
    }

    // RESTART-ONLY: an adapter swap needs a fresh worker process; the
    // persisted entry is only read at boot.
    if adapter_changed(&old, &new) {
        tracing::warn!("state: `adapter` changed; restart-tier — applies at the next worker start");
    }

    tracing::info!("state configuration reloaded");
}

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: Some(timeout_ms),
                }
                .namespace("default"),
            )
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
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/config-client/tests/support/is_not_found_cases.rs"
    ));

    /// The missing-entry classifier only seeds on the configuration worker's
    /// standalone `NOT_FOUND` envelope; every unrelated failure or compound
    /// code propagates instead of clobbering a stored value with a default.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(super::is_not_found);
    }

    /// Preserve state settings when errors do not carry the exact missing-entry code.
    #[test]
    fn missing_entry_detection_does_not_mask_service_failures() {
        assert!(super::is_not_found("NOT_FOUND"));
        assert!(super::is_not_found(
            "remote error (NOT_FOUND): configuration not found"
        ));
        assert!(super::is_not_found(
            "configuration::get failed after 3 attempts: remote error (NOT_FOUND): missing"
        ));
        assert!(!super::is_not_found("function_not_found"));
        assert!(!super::is_not_found("statement_not_found"));
        assert!(!super::is_not_found("RESOURCE_NOT_FOUND"));
        assert!(!super::is_not_found(
            "remote error (ADAPTER_ERROR): NOT_FOUND"
        ));
        assert!(!super::is_not_found(
            "remote error (OTHER): remote error (NOT_FOUND): nested"
        ));
        assert!(!super::is_not_found(
            "configuration::get failed after 3 attempts: function_not_found"
        ));
    }

    use super::*;
    use serde_json::json;

    fn cfg(v: serde_json::Value) -> StateConfig {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn cadence_rebuild_only_when_save_interval_changes() {
        let a = cfg(json!({"save_interval_ms": 5000}));
        let b = cfg(json!({"save_interval_ms": 750}));
        assert!(cadence_changed(&a, &b));
        assert!(!cadence_changed(&a, &a));
        assert!(!cadence_changed(
            &StateConfig::default(),
            &StateConfig::default()
        ));
    }

    #[test]
    fn adapter_change_is_restart_tier() {
        let kv = cfg(json!({"adapter": {"name": "kv"}}));
        let redis = cfg(json!({"adapter": {"name": "redis"}}));
        assert!(adapter_changed(&kv, &redis));
        assert!(!adapter_changed(&kv, &kv));
        // adapter-inner config change is also restart-tier (builtin parity:
        // `old.adapter != new.adapter` compares the whole entry).
        let kv_file =
            cfg(json!({"adapter": {"name": "kv", "config": {"store_method": "file_based"}}}));
        assert!(adapter_changed(&kv, &kv_file));
    }
}
