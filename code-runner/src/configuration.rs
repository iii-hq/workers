//! Integration with the `configuration` worker (docs/sops/configuration.md,
//! Tier 1) — register the JSON Schema + optional seed at boot, read the
//! authoritative (env-expanded) value, and bind a `configuration` trigger so
//! `configuration:updated` re-fetches and swaps the snapshot. Mirrors
//! [`browser`](../../browser/src/configuration.rs).
//!
//! The output caps and timeout knobs are read per call and hot-apply on the
//! swap; the engine-structural fields are boot-captured, so the reload
//! handler warns when a change to them will only apply at the next restart
//! (see `CodeRunnerConfig::restart_required`).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{CodeRunnerConfig, SharedConfig};

pub const CONFIG_ID: &str = "code-runner";

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
/// Internal hot-reload hook; denied to agents in iii-permissions.yaml and
/// seeded into the runtime-id registry (functions::seeded_ids) so a guest
/// `register_function` cannot claim it.
pub const CONFIG_FN_ID: &str = "code-runner::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register execution-policy metadata and seed only a confirmed empty configuration entry.
pub async fn register_config(
    iii: &IIIClient,
    seed: Option<&CodeRunnerConfig>,
) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "code-runner",
        "description": "Runtime limits for in-process Node.js/Python: output caps and timeouts \
                        (hot-reload), plus runtime-count, memory, and scratch limits (applied \
                        at worker restart).",
        "schema": CodeRunnerConfig::json_schema(),
        "metadata": { "ui_form": CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = seed.cloned().unwrap_or_default().to_json();
    ensure_configuration(iii, payload).await
}

pub async fn fetch_config(iii: &IIIClient) -> Result<CodeRunnerConfig, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => CodeRunnerConfig::from_json(&v),
        _ => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(CodeRunnerConfig::default())
        }
    }
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

/// `Ok(None)` when the entry does not exist yet. The engine's missing-entry
/// codes vary in case, so match case-insensitively.
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

/// Register the internal reload hook and bind the `configuration` trigger.
/// Call LAST in boot, after every function and the console UI exist.
pub fn register_config_trigger(iii: &IIIClient, config: SharedConfig) -> Result<(), Error> {
    let cfg = config.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_req: OnConfigChangeRequest| {
            let cfg = cfg.clone();
            let engine = engine.clone();
            async move {
                refresh(&engine, &cfg).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload code-runner settings from the authoritative configuration on \
             change. Output caps and timeouts hot-apply; runtime/memory/scratch limits apply \
             at the next worker restart.",
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

/// Serializes every refresh's fetch→store. The fetch happens INSIDE the
/// lock, so whichever refresh stores later also fetched later — a slow, older
/// `configuration::get` response can never overwrite a newer snapshot.
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Re-fetch the AUTHORITATIVE value and swap the snapshot. The trigger
/// payload is deliberately ignored: `on-config-change` is a discoverable bus
/// function, and trusting a caller-supplied value would let anyone inject
/// config without updating persisted state.
///
/// Called by the `configuration` trigger, and once more at the end of boot:
/// an update landing between the boot fetch and the trigger registration
/// fires into nothing, and without the boot refresh it would stay invisible
/// until the NEXT update or a restart.
pub async fn refresh(iii: &IIIClient, config: &SharedConfig) {
    let _serialized = REFRESH_LOCK.lock().await;
    match fetch_config(iii).await {
        Ok(next) => {
            if config.load().restart_required(&next) {
                tracing::warn!(
                    "configuration changed a boot-captured field (runtimes/memory/scratch); \
                     the change is saved but applies at the next worker restart"
                );
            }
            config.store(Arc::new(next));
            tracing::info!("code-runner configuration reloaded");
        }
        Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
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

    /// The store()/load() path the reload handler uses: a swap must be
    /// visible to the next per-call read.
    #[test]
    fn snapshot_swap_is_visible_to_readers() {
        let shared = CodeRunnerConfig::default().into_shared();
        assert_eq!(shared.load().max_result_bytes, 32_768);
        shared.store(Arc::new(CodeRunnerConfig {
            max_result_bytes: 7,
            ..CodeRunnerConfig::default()
        }));
        assert_eq!(shared.load().max_result_bytes, 7);
    }
}
