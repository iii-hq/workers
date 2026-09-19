//! Integration with the builtin `configuration` worker.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::boot::{self, BootParts, ConfigCell, SchedulerCell};
use crate::config::CronConfig;
use crate::locks;
use crate::scheduler::Scheduler;

pub const CONFIG_ID: &str = "cron";

/// Process-stable entry identity; the form family remains CONFIG_ID.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONFIG_ID.to_string())
    })
    .as_str()
}
const CONFIG_FN_ID: &str = "cron::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
const CONFIG_BUS_TIMEOUT_MS: u64 = 10_000;

/// Refresh scheduling metadata, supplying defaults only for an empty configuration entry.
pub async fn register_config(iii: &IIIClient, seed: Option<&CronConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Cron",
        "description": "Cron scheduler settings - lock backend for multi-instance mutual exclusion (local or redis).",
        "schema": CronConfig::json_schema(),
        "metadata": { "ui_form": CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override
    // (even `false`/`0`/`""`) is preserved without a client-side
    // read-then-register race.
    let seed = seed.cloned().unwrap_or_default().normalized();
    payload["initial_value"] = seed.to_json();
    match trigger_with_retry(iii, "configuration::ensure", payload, CONFIG_BUS_TIMEOUT_MS).await {
        Ok(_) => Ok(()),
        Err(e) if is_function_not_found(&e) => Err(ENSURE_UNAVAILABLE.to_string()),
        Err(e) => Err(e),
    }
}

/// Parse the authoritative scheduling settings, with a first-boot fallback if unset.
pub async fn fetch_config(iii: &IIIClient) -> Result<CronConfig, String> {
    match try_get_config_value(iii).await? {
        Some(value) if !value.is_null() => CronConfig::from_json(&value),
        _ => {
            tracing::info!(
                id = config_id(),
                "no configuration value stored; using built-in default"
            );
            Ok(CronConfig::default())
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

/// Query the instance's assigned entry and preserve non-entry errors for the caller.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(
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

/// Reconcile the cron runtime when the resolved configuration entry changes.
pub fn register_config_trigger(iii: &Arc<IIIClient>, parts: BootParts) -> Result<(), Error> {
    let engine = iii.clone();
    let parts_for_fn = parts.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: ConfigChangeRequest| {
            let engine = engine.clone();
            let parts = parts_for_fn.clone();
            async move {
                on_config_change(&engine, &parts).await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description("Internal: reload cron configuration from the authoritative store on change.")
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

async fn on_config_change(iii: &Arc<IIIClient>, parts: &BootParts) {
    let _guard = parts.apply_lock.lock().await;

    let next = match fetch_config(iii).await {
        Ok(cfg) => cfg.normalized(),
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let current = parts.config.read().await.clone();
    if !swap_needed(&current, &next) {
        *parts.config.write().await = next;
        tracing::info!("cron configuration reloaded without lock-backend swap");
        return;
    }

    if let Err(e) = swap_scheduler(iii, &parts.scheduler, &parts.config, next).await {
        tracing::error!(error = %e, "cron lock-backend swap failed; keeping previous scheduler");
    }
}

async fn swap_scheduler(
    iii: &Arc<IIIClient>,
    scheduler_cell: &SchedulerCell,
    config_cell: &ConfigCell,
    next: CronConfig,
) -> anyhow::Result<()> {
    let lock = locks::build_lock(&next).await?;
    let old = scheduler_cell.read().await.clone();
    let specs = old.job_specs().await;
    let new_scheduler = Arc::new(Scheduler::new(lock, boot::sdk_invoker(iii.clone())));

    old.shutdown().await;
    for spec in specs {
        if let Err(e) = new_scheduler.register(spec.clone()).await {
            tracing::error!(
                trigger_id = %spec.trigger_id,
                error = %e,
                "failed to re-register cron job after lock-backend swap"
            );
        }
    }

    *scheduler_cell.write().await = new_scheduler;
    *config_cell.write().await = next;
    tracing::info!("cron configuration reloaded with lock-backend swap");
    Ok(())
}

fn swap_needed(current: &CronConfig, next: &CronConfig) -> bool {
    current.effective_adapter_name() != next.effective_adapter_name()
        || current.adapter.as_ref().and_then(|a| a.config.clone())
            != next.adapter.as_ref().and_then(|a| a.config.clone())
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

/// `true` only when the error carries the configuration worker's standalone
/// `NOT_FOUND` entry code, identified by the outermost `remote error (<code>)` envelope code rather than a substring or token scan of the message, so
/// a compound code such as `RESOURCE_NOT_FOUND`/`STATEMENT_NOT_FOUND` or the
/// engine's lowercase missing-FUNCTION code `function_not_found` still
/// propagates as a failure instead of being read as "nothing stored yet".
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

    use super::*;

    #[test]
    fn swap_needed_only_when_adapter_changes() {
        let local: CronConfig = serde_yaml::from_str("{adapter: {name: local}}").unwrap();
        let redis: CronConfig = serde_yaml::from_str("{adapter: {name: redis}}").unwrap();
        assert!(!swap_needed(&local, &local));
        assert!(swap_needed(&local, &redis));
        assert!(
            !swap_needed(&CronConfig::default(), &local),
            "default IS local"
        );
    }
}
