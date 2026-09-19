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

    if should_seed_default(iii).await? {
        payload["initial_value"] = seed.cloned().unwrap_or_default().to_json();
    }

    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    match try_get_value(iii).await? {
        Some(value) if !value.is_null() => WorkerConfig::from_json(&value),
        _ => Ok(WorkerConfig::default()),
    }
}

async fn should_seed_default(iii: &IIIClient) -> Result<bool, String> {
    Ok(try_get_value(iii)
        .await?
        .is_none_or(|value| value.is_null()))
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
    match error.split_once("remote error (") {
        Some((_, rest)) => rest
            .split_once(')')
            .is_some_and(|(code, _)| code == "NOT_FOUND"),
        None => error.trim() == "NOT_FOUND",
    }
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
