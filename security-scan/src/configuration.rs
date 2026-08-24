use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::schema_for;
use serde_json::{json, Value};

use crate::{manifest, SecurityScanError, WorkerConfig};

pub const CONFIG_ID: &str = "security-scan";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

pub fn shipped_config() -> WorkerConfig {
    serde_json::from_value(manifest::build_manifest().default_config)
        .expect("security-scan manifest config must match WorkerConfig")
}

pub async fn register_and_fetch(iii: &IIIClient) -> Result<WorkerConfig, SecurityScanError> {
    let initial_value = match try_get_value(iii).await? {
        Some(value) if !value.is_null() => None,
        _ => Some(serde_json::to_value(shipped_config()).map_err(|error| {
            SecurityScanError::Dependency(format!("could not serialize shipped config: {error}"))
        })?),
    };

    let schema = serde_json::to_value(schema_for!(WorkerConfig)).map_err(|error| {
        SecurityScanError::Dependency(format!("could not serialize config schema: {error}"))
    })?;
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Security Scan",
        "description": "Operator repository allowlist and bounded read-only Harness analysis settings.",
        "schema": schema,
    });
    if let Some(initial_value) = initial_value {
        payload["initial_value"] = initial_value;
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;

    let value = try_get_value(iii)
        .await?
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            SecurityScanError::Dependency(format!(
                "configuration::{CONFIG_ID} was not available after registration"
            ))
        })?;
    let config: WorkerConfig = serde_json::from_value(value).map_err(|error| {
        SecurityScanError::Dependency(format!("could not parse {CONFIG_ID} config: {error}"))
    })?;
    config.validate()?;
    Ok(config)
}

async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, SecurityScanError> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(response) => response.get("value").cloned().map(Some).ok_or_else(|| {
            SecurityScanError::Dependency("configuration::get returned no `value` field".into())
        }),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, SecurityScanError> {
    let mut last_error = None;
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(CONFIG_TIMEOUT_MS),
            })
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) => {
                last_error = Some(error.to_string());
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * u64::from(attempt),
                    ))
                    .await;
                }
            }
        }
    }
    Err(SecurityScanError::Dependency(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {}",
        last_error.unwrap_or_else(|| "unknown error".into())
    )))
}

fn is_not_found(error: &SecurityScanError) -> bool {
    let message = error.to_string().to_ascii_uppercase();
    message.contains("NOT_FOUND") || message.contains("NOT FOUND")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_config_is_idle_and_valid() {
        let config = shipped_config();
        assert!(config.repositories.is_empty());
        assert!(config.analysis.model.is_empty());
        config.validate().expect("idle defaults validate");
    }

    #[test]
    fn config_schema_keeps_nested_definitions() {
        let schema = serde_json::to_value(schema_for!(WorkerConfig)).expect("schema serializes");
        assert!(schema["definitions"].is_object());
        assert!(schema["properties"]["analysis"].is_object());
        assert!(schema["definitions"]["RepositoryConfigV1"]["properties"]["github"].is_object());
        assert!(schema["definitions"]["RepositoryConfigV1"]["properties"]["schedule"].is_object());
        let required = schema["definitions"]["RepositoryConfigV1"]["required"]
            .as_array()
            .expect("repository required fields");
        assert!(!required.iter().any(|field| field == "github"));
        assert!(!required.iter().any(|field| field == "schedule"));
    }
}
