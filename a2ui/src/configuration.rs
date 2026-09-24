//! Path B integration with the `configuration` worker.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "a2ui";

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
pub const CONFIG_FN_ID: &str = "a2ui::on-config-change";
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Advisory id only. The handler always re-fetches authoritative state.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Refresh the schema; supply the seed or defaults only after a confirmed empty entry.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "A2UI",
        "description": "A2UI composer routing, correction budget, per-session surface limits, and Console action forwarding.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = seed.cloned().unwrap_or_default().to_json();
    ensure_configuration(iii, payload).await
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Subscribe to updates of the resolved entry and reload authoritative values.
pub fn register_config_trigger(iii: &IIIClient, cell: ConfigCell) -> Result<(), Error> {
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(cfg) => {
                        apply_config(&cell, cfg).await;
                        tracing::info!("A2UI configuration reloaded");
                    }
                    Err(error) => tracing::error!(%error, "failed to reload A2UI configuration"),
                }
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload the A2UI worker from authoritative configuration after an update.",
        )
        .metadata(json!({ "internal": true })),
    );
    iii.register_trigger(RegisterTriggerInput::new(
        "configuration",
        CONFIG_FN_ID,
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"]
        }),
    ))?;
    Ok(())
}

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        trigger_with_retry(iii, function, payload)
    })
    .await
}

#[path = "../../crates/config-client/src/initialization.rs"]
mod initialization;

/// Require a stored value for the entry assigned to this worker instance.
async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{}` not found", config_id()))
}

/// Map only the configuration service's missing-entry code to absence.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({"id": config_id()})).await {
        Ok(response) => Ok(response.get("value").cloned()),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Inspect the SDK error code without matching a compound code or its message.
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

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_error = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.into(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: None,
                }
                .namespace("default"),
            )
            .await
        {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error.to_string();
                if matches!(&error, iii_sdk::errors::Error::Remote { code, .. } if code == "function_not_found" || code == "NOT_FOUND")
                {
                    return Err(last_error);
                }
                if is_not_found(&last_error) {
                    return Err(last_error);
                }
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
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_error}"
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

    use super::*;

    /// Entry absence permits seeding; routing failures and unrelated codes must propagate.
    #[test]
    fn missing_entry_detection_does_not_mask_service_failures() {
        assert!(is_not_found(
            "remote error (NOT_FOUND): configuration 'a2ui' not found"
        ));
        assert!(is_not_found(
            "configuration::get failed after 3 attempts: remote error (NOT_FOUND): missing"
        ));
        assert!(!is_not_found("remote error (ADAPTER_ERROR): NOT_FOUND"));
        assert!(!is_not_found(
            "remote error (OTHER): remote error (NOT_FOUND): nested"
        ));
        assert!(!is_not_found("function_not_found"));
        assert!(!is_not_found("statement_not_found"));
        assert!(!is_not_found("timed out"));
        assert!(!is_not_found("RESOURCE_NOT_FOUND"));
        assert!(!is_not_found("STATEMENT_NOT_FOUND"));
        assert!(!is_not_found("NOT_FOUND_EXTRA"));
    }

    #[tokio::test]
    async fn config_snapshot_swaps() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        apply_config(
            &cell,
            WorkerConfig {
                max_surfaces_per_session: 3,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.max_surfaces_per_session, 3);
    }
}
