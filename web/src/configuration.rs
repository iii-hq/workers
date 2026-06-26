//! Integration with the `configuration` worker: register a JSON Schema +
//! seed at boot, read the authoritative (env-expanded) value, and bind a
//! `configuration` trigger so `configuration:updated` re-fetches and applies
//! the change. All WebConfig fields hot-reload (no topology partition).

use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{SharedConfig, WebConfig};

pub const CONFIG_ID: &str = "web";
const CONFIG_FN_ID: &str = "web::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

#[derive(Clone)]
pub struct SharedState {
    pub config: SharedConfig,
}

pub async fn register_config(iii: &IIIClient, seed: Option<&WebConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "web",
        "description": "Timeouts, byte caps, user-agent, and loopback policy for the web::fetch worker.",
        "schema": WebConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default(iii).await? {
        payload["initial_value"] = WebConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WebConfig, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => WebConfig::from_json(&v),
        _ => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(WebConfig::default())
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

async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(state: &SharedState, cfg: WebConfig) {
    state.config.store(std::sync::Arc::new(cfg));
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeRequest {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeResponse {
    ok: bool,
}

pub fn register_config_trigger(iii: &IIIClient, state: SharedState) -> Result<(), Error> {
    let st = state.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_req: OnConfigChangeRequest| {
            let st = st.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &st).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload web settings from the authoritative configuration on change.",
        ),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({ "configuration_id": CONFIG_ID, "event_types": ["configuration:updated"] }),
        metadata: None,
    })?;
    Ok(())
}

async fn on_config_change(iii: &IIIClient, state: &SharedState) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(state, cfg).await;
            tracing::info!("web configuration reloaded");
        }
        Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(CONFIG_TIMEOUT_MS),
            })
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
