//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, hot-reload on change (Path B;
//! mirrors the memory worker).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::WorkerConfig;
use crate::functions::ConfigCell;

pub const DEFAULT_CONFIG_ID: &str = "memory-consolidate";

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
const CONFIG_FN_ID: &str = "memory-consolidate::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register consolidation settings, seeding only a confirmed missing or null value.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Memory Consolidate",
        "description": "Scheduled memory hygiene: dedup interval, dry-run mode, bank allowlist, \
                        and the per-pass supersede cap.",
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

#[path = "../../crates/config-client/src/initialization.rs"]
mod initialization;

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        trigger_configuration_with_retry(iii, function, payload)
    })
    .await
}

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

/// `Ok(None)` = the entry does not exist. A successful reply MUST carry
/// `value` — a malformed response must not seed defaults over an intended
/// configuration.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => resp
            .get("value")
            .cloned()
            .map(Some)
            .ok_or_else(|| "configuration::get returned no `value` field".to_string()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

pub fn register_config_trigger(iii: &IIIClient, cell: ConfigCell) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            async move {
                let cfg = fetch_config(&engine)
                    .await
                    .map_err(|e| Error::Handler(format!("config-change fetch failed: {e}")))?;
                *cell.write().await = Arc::new(cfg);
                tracing::info!("memory-consolidate configuration reloaded");
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload memory-consolidate from the authoritative configuration when \
             it changes.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
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
                    timeout_ms: Some(CONFIG_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                if matches!(&e, iii_sdk::errors::Error::Remote { code, .. } if code == "function_not_found" || code == "NOT_FOUND")
                {
                    return Err(last_err);
                }
                if attempt < CONFIG_RETRIES {
                    tracing::warn!(function_id, attempt, error = %last_err, "configuration RPC failed; retrying");
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

    /// The shared missing-entry contract, exercised against this worker's real
    /// private `is_not_found` so only a genuine `NOT_FOUND` envelope seeds.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(super::is_not_found);
    }

    /// Only the entry code authorizes seeding; compound codes and message text do not.
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
}
