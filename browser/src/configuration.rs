//! Integration with the `configuration` worker: register a JSON Schema +
//! seed at boot, read the authoritative (env-expanded) value, and bind a
//! `configuration` trigger so `configuration:updated` re-fetches and applies
//! the change. Caps and timeouts hot-reload; `executable`/`headless`/viewport
//! apply to sessions started after the change.

use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{SharedConfig, WorkerConfig};

pub const DEFAULT_CONFIG_ID: &str = "browser";

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
const CONFIG_FN_ID: &str = "browser::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Refresh the browser schema; seed only after confirming the entry has no stored value.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "browser",
        "description": "Session limits, buffers, timeouts, viewport, and executable for the browser worker.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // A seed initializes an absent entry; it never replaces a Compose override.
    if should_seed_default(iii).await? {
        payload["initial_value"] = seed
            .map(|value| value.to_json())
            .unwrap_or_else(|| WorkerConfig::default().to_json());
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => WorkerConfig::from_json(&v),
        _ => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(WorkerConfig::default())
        }
    }
}

async fn should_seed_default(iii: &IIIClient) -> Result<bool, String> {
    match try_get_value(iii).await? {
        None => Ok(true),
        Some(v) if v.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

/// Treat only the missing-entry code as absence and propagate dependency failures.
async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeRequest {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeResponse {
    ok: bool,
}

pub fn register_config_trigger(
    iii: &IIIClient,
    config: SharedConfig,
    guidance: crate::scrapling::GuidanceState,
) -> Result<(), Error> {
    let cfg = config.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_req: OnConfigChangeRequest| {
            let cfg = cfg.clone();
            let engine = engine.clone();
            let guidance = guidance.clone();
            async move {
                on_config_change(&engine, &cfg, &guidance).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload browser settings from the authoritative configuration on change.",
        )
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({ "configuration_id": config_id(), "event_types": ["configuration:updated"] }),
    ))?;
    Ok(())
}

async fn on_config_change(
    iii: &IIIClient,
    config: &SharedConfig,
    guidance: &crate::scrapling::GuidanceState,
) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            crate::scrapling::adaptive::configure_quota(cfg.scrapling.adaptive_quota());
            let inject_guidance = cfg.scrapling.inject_guidance;
            config.store(std::sync::Arc::new(cfg));
            // Hot-apply: flipping browser.scrapling.inject_guidance in the
            // console binds/unbinds the pre-generate guidance hook live.
            crate::scrapling::apply_guidance(iii, guidance, inject_guidance);
            tracing::info!("browser configuration reloaded");
        }
        Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
    }
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
                if attempt < CONFIG_RETRIES {
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_err,
                        "configuration RPC failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}
