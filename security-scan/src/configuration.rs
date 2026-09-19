use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::schema_for;
use serde_json::{json, Value};

use crate::{manifest, SecurityScanError, WorkerConfig};

pub const CONFIG_ID: &str = "security-scan";

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
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

pub fn shipped_config() -> WorkerConfig {
    serde_json::from_value(manifest::build_manifest().default_config)
        .expect("security-scan manifest config must match WorkerConfig")
}

/// Publish schema and identity, seed an empty entry, then load the authoritative scan settings.
pub async fn register_and_fetch(iii: &IIIClient) -> Result<WorkerConfig, SecurityScanError> {
    iii_console_ui::register_configuration_identity(iii, "security-scan", config_id());

    let schema = serde_json::to_value(schema_for!(WorkerConfig)).map_err(|error| {
        SecurityScanError::Dependency(format!("could not serialize config schema: {error}"))
    })?;
    let mut payload = json!({
        "id": config_id(),
        "name": "Security Scan",
        "description": "Operator repository allowlist and bounded read-only Harness analysis settings.",
        "schema": schema,
        "metadata": { "ui_form": CONFIG_ID },
    });
    payload["initial_value"] = serde_json::to_value(shipped_config()).map_err(|error| {
        SecurityScanError::Dependency(format!("could not serialize shipped config: {error}"))
    })?;
    match trigger_with_retry(iii, "configuration::ensure", payload).await {
        Ok(_) => {}
        Err(error) if is_function_not_found(&error) => {
            // Fail CLOSED: an engine without atomic configuration::ensure must not
            // launch security-scan on shipped defaults over a stored allowlist.
            tracing::error!(
                %error,
                "configuration::ensure unavailable; upgrade engine with atomic configuration initialization support"
            );
            std::process::exit(1);
        }
        Err(error) => return Err(error),
    }

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

/// `true` when the dependency error is the engine's lowercase missing-FUNCTION
/// envelope `function_not_found` — an engine that predates atomic
/// `configuration::ensure`. Distinct from [`is_not_found`] (the configuration
/// worker's `NOT_FOUND` entry code); a local `InvalidRequest` is never one.
fn is_function_not_found(error: &SecurityScanError) -> bool {
    match error {
        SecurityScanError::Dependency(message) => is_missing_function_message(message),
        SecurityScanError::InvalidRequest(_) => false,
    }
}

/// Peel the one retry wrapper, then require the `function_not_found` envelope at
/// the very start so a stray token in an unrelated message still propagates.
fn is_missing_function_message(message: &str) -> bool {
    const RETRY_WRAPPER: &str = "configuration::ensure failed after 3 attempts: ";
    let raw = message.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "function_not_found"
        || raw == "remote error (function_not_found):"
        || raw.starts_with("remote error (function_not_found): ")
}

pub async fn register_and_fetch_until_ready(iii: &IIIClient) -> WorkerConfig {
    retry_until_ready(
        || register_and_fetch(iii),
        Duration::from_millis(CONFIG_RETRY_BACKOFF_MS),
    )
    .await
}

async fn retry_until_ready<T, F, Fut>(mut operation: F, delay: Duration) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SecurityScanError>>,
{
    loop {
        match operation().await {
            Ok(value) => return value,
            Err(error) => {
                tracing::warn!(%error, "security-scan configuration unavailable; retrying");
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Missing entries may be seeded; dependency failures must propagate without writes.
async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, SecurityScanError> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": config_id() })).await {
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
            .trigger(
                TriggerRequest {
                    function_id: function_id.into(),
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

/// Inspect the remote envelope code rather than words inside an unrelated failure message.
fn is_not_found(error: &SecurityScanError) -> bool {
    // Only a dependency (RPC) failure can carry the configuration worker's
    // NOT_FOUND envelope. A local `InvalidRequest` is our own validation error
    // and must never be read as "nothing stored yet", even if its message
    // happens to contain the token. Match the variant first, then inspect its
    // inner message rather than the `Display` string of the whole error.
    match error {
        SecurityScanError::Dependency(message) => is_missing_entry_message(message),
        SecurityScanError::InvalidRequest(_) => false,
    }
}

/// Anchor on the SDK's own rendering of a dependency failure: it prints as
/// `remote error ({code}): {message}`, and this worker wraps a retried get as
/// `configuration::get failed after CONFIG_RETRIES attempts: {err}`. Peel that
/// one wrapper (never a foreign one or a different attempt count) and then
/// require the NOT_FOUND envelope at the very start, so a NOT_FOUND token
/// buried in an unrelated message or wrapper still propagates as a failure.
fn is_missing_entry_message(message: &str) -> bool {
    const RETRY_WRAPPER: &str = "configuration::get failed after 3 attempts: ";
    const _: () = assert!(CONFIG_RETRIES == 3);
    let raw = message.trim();
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

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Missing-entry classification runs on the stringified error: only the
    /// configuration worker's standalone `NOT_FOUND` code inside the SDK's
    /// `remote error (<code>)` envelope maps to an absent entry. A different
    /// code whose message mentions NOT_FOUND, a nested envelope, and compound
    /// codes all propagate instead of being read as "nothing stored yet".
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        // Only a dependency failure carries the envelope; the classifier reads
        // its inner message, so the shared string contract applies to it.
        assert_missing_entry_contract(|message| {
            is_not_found(&SecurityScanError::Dependency(message.to_string()))
        });
        // A local validation error is never a missing entry, even when its
        // message contains the envelope verbatim.
        assert!(!is_not_found(&SecurityScanError::InvalidRequest(
            "remote error (NOT_FOUND): not a config read".into()
        )));
    }

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
        assert!(schema["properties"]["archive"].is_object());
        let required = schema["definitions"]["RepositoryConfigV1"]["required"]
            .as_array()
            .expect("repository required fields");
        assert!(!required.iter().any(|field| field == "github"));
        assert!(!required.iter().any(|field| field == "schedule"));
    }

    #[tokio::test]
    async fn transient_configuration_failure_is_retried_before_use() {
        let attempts = AtomicUsize::new(0);
        let value = retry_until_ready(
            || {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt < 2 {
                        Err(SecurityScanError::Dependency("not ready".into()))
                    } else {
                        Ok("authoritative")
                    }
                }
            },
            Duration::ZERO,
        )
        .await;
        assert_eq!(value, "authoritative");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
