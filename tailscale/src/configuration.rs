use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{SharedConfig, WorkerConfig};

pub const CONFIG_ID: &str = "tailscale";

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
const CONFIG_FUNCTION_ID: &str = "tailscale::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Refresh the schema and initialize only an entry whose stored value is absent or null.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Tailscale",
        "description": "Local Console target, CLI path, default HTTPS port, public Funnel policy, and command timeout.",
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
    match try_get_value(iii).await? {
        Some(value) if !value.is_null() => WorkerConfig::from_json(&value),
        _ => Ok(WorkerConfig::default()),
    }
}

#[path = "../../crates/config-client/src/initialization.rs"]
mod initialization;

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        trigger_with_retry(iii, function, payload)
    })
    .await
}

/// Read the authoritative entry without converting service errors into absence.
async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({"id": config_id()})).await {
        Ok(response) => Ok(response.get("value").cloned()),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeInput {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeOutput {
    ok: bool,
}

/// Reapply Tailscale settings when this instance's resolved configuration entry changes.
pub fn register_config_trigger(iii: &IIIClient, config: SharedConfig) -> Result<(), Error> {
    let shared = config.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FUNCTION_ID,
        RegisterFunction::new_async(move |_input: OnConfigChangeInput| {
            let shared = shared.clone();
            let engine = engine.clone();
            async move {
                let ok = match fetch_config(&engine).await {
                    Ok(next) => {
                        shared.store(std::sync::Arc::new(next));
                        tracing::info!("tailscale configuration reloaded");
                        true
                    }
                    Err(error) => {
                        tracing::error!(%error, "keeping the previous tailscale configuration");
                        false
                    }
                };
                Ok::<_, Error>(OnConfigChangeOutput { ok })
            }
        })
        .description("Internal: reload Tailscale settings after a configuration update.")
        .metadata(json!({"internal": true})),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FUNCTION_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"]
        }),
    ))?;
    Ok(())
}

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
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: Some(CONFIG_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) => {
                last_error = error.to_string();
                if matches!(&error, iii_sdk::errors::Error::Remote { code, .. } if code == "function_not_found" || code == "NOT_FOUND")
                {
                    return Err(last_error);
                }
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
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
}
