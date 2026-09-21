//! Integration with the `configuration` worker: register the schema, fetch
//! the authoritative value at boot, and hot-reload on change (Tier 1:
//! ConfigCell snapshot swap plus a targeted cron re-bind when
//! `prune_schedule` changes). Mirrors `approval-gate`.
//!
//! `configuration` is a REQUIRED boot dependency: a failed register/fetch
//! aborts startup.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler: take a
/// `read().await`, clone the inner `Arc` out, drop the lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const DEFAULT_CONFIG_ID: &str = "worktree";

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
const CONFIG_FN_ID: &str = "worktree::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the schema. The optional seed or built-in default is installed
/// only when no stored value exists; live configuration always takes precedence.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Worktree",
        "description": "Git worktree lifecycle settings: the managed worktree root, \
                        branch prefix, prune schedule and expiry, land queue name, \
                        retry bound, and git/test timeouts.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = seed
        .map(|value| value.to_json())
        .unwrap_or_else(|| WorkerConfig::default().to_json());
    ensure_configuration(iii, payload).await
}

/// Forward `payload` to the engine's atomic `configuration::ensure`. Fails
/// CLOSED against an engine that predates it (`function_not_found`): surface
/// the upgrade-required error instead of falling back to the legacy
/// read-then-`register` seed, which could clobber a stored override.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    match trigger_configuration_with_retry(iii, "configuration::ensure", payload).await {
        Ok(_) => Ok(()),
        Err(e) if is_function_not_found(&e) => Err(ENSURE_UNAVAILABLE.to_string()),
        Err(e) => Err(e),
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

/// Read the live `worktree` configuration (already env-expanded by the
/// configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in defaults");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
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

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// `Ok(None)` when the entry does not exist; missing-entry codes vary in
/// case across engine versions, so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// The live prune cron binding, re-bindable on `prune_schedule` change.
/// Register the new binding first, then unregister the old (fail-safe
/// overlap).
#[derive(Clone, Default)]
pub struct CronSlot {
    inner: Arc<Mutex<Option<Trigger>>>,
}

impl CronSlot {
    pub fn rebind(&self, iii: &IIIClient, schedule: &str) {
        // `expression` is the binding key the cron trigger type consumes
        // (engine built-in and the standalone cron worker alike), matching
        // how the harness binds its own sweep.
        let new = match iii.register_trigger(RegisterTriggerInput::new(
            "cron".to_string(),
            "worktree::prune".to_string(),
            json!({ "expression": schedule }),
        )) {
            Ok(trigger) => {
                tracing::info!(schedule, "prune cron binding registered");
                trigger
            }
            Err(e) => {
                // Keep the working binding: swapping in a failed registration
                // would unregister the live cron and leave the sweep unbound.
                tracing::warn!(error = %e, schedule, "prune cron binding failed; keeping previous binding");
                return;
            }
        };
        let old = {
            let mut slot = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            slot.replace(new)
        };
        if let Some(old) = old {
            old.unregister();
        }
    }
}

/// Internal `worktree::on-config-change` trigger payload. The handler
/// re-fetches the authoritative configuration and ignores this payload, so
/// a direct call can never inject config.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `worktree::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. Registered here (not in `functions::register_all`) but still
/// pinned by `surface::catalog()`.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    cron: CronSlot,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    let cron_for_fn = cron.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            let cron = cron_for_fn.clone();
            async move {
                on_config_change(&engine, &cell, &cron).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the worktree worker from the authoritative \
             configuration when it changes — swaps the per-call snapshot and \
             re-binds the prune cron on a schedule change.",
        )
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

/// Reload from the AUTHORITATIVE configuration; keep the previous snapshot
/// on any failure path.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, cron: &CronSlot) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: fetch failed; keeping previous configuration"
            );
            return;
        }
    };
    let previous_signature = cell.read().await.boot_signature();
    let structural = cfg.boot_signature() != previous_signature;
    let schedule = cfg.prune_schedule.clone();
    apply_config(cell, cfg).await;
    if structural {
        cron.rebind(iii, &schedule);
    }
    tracing::info!(structural, "worktree configuration reloaded");
}

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: None,
                }
                .namespace("default"),
            )
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

    /// Only a missing entry permits defaults; unrelated failures preserve stored worktree policy.
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

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.max_land_retries, 3);
        let tuned = WorkerConfig {
            max_land_retries: 9,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.max_land_retries, 9);
    }
}
