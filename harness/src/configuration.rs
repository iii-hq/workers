//! Integration with the `configuration` worker — register the schema, fetch
//! the authoritative value at boot, and hot-reload it when it changes. Tier 1
//! (ConfigCell + targeted re-observe), mirroring `context-manager` /
//! `approval-gate`.
//!
//! Every field hot-reloads — nothing requires a restart:
//!
//! - `sweep_expression` is the one STRUCTURAL field (the `cron::on-tick`
//!   observer that drives the pending-call sweep). On a change the handler
//!   observes the new expression, then disposes the old observer — a
//!   fail-safe overlap; the sweep is idempotent — keeping the
//!   [`ObserverHandle`] in [`LifecycleObservers`].
//! - Every OTHER field is a per-call tuning knob read from the live snapshot
//!   via [`Deps::cfg`](crate::deps::Deps::cfg); a change swaps the snapshot.
//!
//! `configuration` is a required boot dependency: a failed register/fetch
//! aborts startup.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::CallRequest;
use iii_sdk::{Deliver, IIIClient, ObserverHandle, RegisterFunction, Source};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;
use crate::functions::sweep_pending::SWEEP_PENDING_ID;
use crate::subscriptions::ON_SESSION_DELETED_ID;

/// Hot-swappable config snapshot shared with every handler.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const DEFAULT_CONFIG_ID: &str = "harness";

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
const CONFIG_FN_ID: &str = "harness::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the schema. The optional seed or built-in default is installed
/// only when no stored value exists; live configuration always takes precedence.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    discard_legacy_harness_config_if_needed(iii).await?;
    drop_stored_provider_identity_prompt_if_needed(iii).await?;

    let mut payload = json!({
        "id": config_id(),
        "name": "Harness",
        "description": "Durable turn-loop settings: default turn cap and pending-call timeout, \
                        sub-agent depth/fan-out budgets, output-contract validation retries, the \
                        webhook-dedupe TTL, per-dependency RPC timeouts, stream coalescing, and \
                        the pending-sweep cron schedule.",
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

/// Initialization only; intentional configuration migrations remain unchanged.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        call_configuration_with_retry(iii, function, payload)
    })
    .await
}
/// Read the live `harness` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
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
    try_get_config_value(iii, false).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// Returns `Ok(None)` when the entry does not exist (codes vary in case).
async fn try_get_config_value(iii: &IIIClient, raw: bool) -> Result<Option<Value>, String> {
    match call_configuration_with_retry(iii, "configuration::get", configuration_get_payload(raw))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

fn configuration_get_payload(raw: bool) -> Value {
    json!({ "id": config_id(), "raw": raw })
}

/// True when the stored value matches the pre-Rust harness entry (permissions
/// block and/or stale provider credentials).
fn is_legacy_harness_value(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    obj.contains_key("permissions") || obj.contains_key("providers")
}

/// Legacy keys present in a stored value (for the discard warning).
fn legacy_harness_keys(value: &Value) -> Vec<&'static str> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    if obj.contains_key("permissions") {
        keys.push("permissions");
    }
    if obj.contains_key("providers") {
        keys.push("providers");
    }
    keys
}

/// Discard a pre-Rust harness configuration entry so strict schema registration
/// can succeed. There is no `configuration::delete`; unlock a permissive schema,
/// replace the value with built-in defaults, then let the caller register the
/// real schema.
async fn discard_legacy_harness_config_if_needed(iii: &IIIClient) -> Result<(), String> {
    let Some(value) = try_get_config_value(iii, false).await? else {
        return Ok(());
    };
    if value.is_null() || !is_legacy_harness_value(&value) {
        return Ok(());
    }

    let dropped = legacy_harness_keys(&value);
    tracing::warn!(
        dropped_keys = ?dropped,
        "legacy pre-Rust harness configuration detected; discarding stored value and \
         re-seeding with built-in turn-loop defaults — provider credentials belong in the \
         llm-router entry, approval defaults belong in the approval-gate entry"
    );

    call_configuration_with_retry(
        iii,
        "configuration::register",
        json!({
            "id": config_id(),
            "name": "Harness",
            "description": "Legacy harness configuration placeholder (discarding pre-Rust entry).",
            "schema": {
                "type": "object",
                "additionalProperties": true
            },
        }),
    )
    .await?;

    call_configuration_with_retry(
        iii,
        "configuration::set",
        json!({
            "id": config_id(),
            "value": WorkerConfig::default().to_json(),
        }),
    )
    .await?;

    Ok(())
}

/// Remove the retired provider-owned prompt setting before re-registering the
/// strict schema. All other stored settings remain intact.
async fn drop_stored_provider_identity_prompt_if_needed(iii: &IIIClient) -> Result<(), String> {
    let Some(value) = try_get_config_value(iii, true).await? else {
        return Ok(());
    };
    let mut migrated = value.clone();
    if !drop_provider_identity_prompt(&mut migrated) {
        return Ok(());
    }

    tracing::info!("removing retired provider_identity_prompt from harness configuration");
    call_configuration_with_retry(
        iii,
        "configuration::register",
        json!({
            "id": config_id(),
            "name": "Harness",
            "description": "Harness configuration migration.",
            "schema": {
                "type": "object",
                "additionalProperties": true
            },
        }),
    )
    .await?;

    let mut expected = value;
    loop {
        let response = call_configuration_with_retry(
            iii,
            "configuration::set",
            json!({ "id": config_id(), "value": migrated }),
        )
        .await?;
        let overwritten = response.get("old_value").cloned().ok_or_else(|| {
            "configuration::set response missing old_value during harness migration".to_string()
        })?;
        let Some(rebased) =
            reconcile_provider_identity_prompt_write(&mut expected, &migrated, overwritten)
        else {
            return Ok(());
        };
        tracing::warn!(
            "harness configuration changed during migration; preserving the intervening update"
        );
        migrated = rebased;
    }
}

fn drop_provider_identity_prompt(value: &mut Value) -> bool {
    value
        .as_object_mut()
        .is_some_and(|settings| settings.remove("provider_identity_prompt").is_some())
}

fn reconcile_provider_identity_prompt_write(
    expected: &mut Value,
    written: &Value,
    mut overwritten: Value,
) -> Option<Value> {
    // `old_value` is atomic with the set. A mismatch means this write replaced
    // a newer snapshot, so restore that snapshot with only the retired key gone.
    if overwritten == *expected {
        return None;
    }
    *expected = written.clone();
    drop_provider_identity_prompt(&mut overwritten);
    Some(overwritten)
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// The cron worker's tick source; its args are `{ expression }`.
const ON_TICK_FN: &str = "cron::on-tick";
/// session-manager's deletion event.
const ON_SESSION_DELETED_FN: &str = "session::on-deleted";
/// The engine's configuration change event.
const ON_CONFIGURATION_UPDATED_FN: &str = "configuration::on-updated";

/// Live observer handles retained for the worker lifetime.
pub struct LifecycleObservers {
    pub sweep: std::sync::Mutex<ObserverHandle>,
    _session_deleted: ObserverHandle,
}

impl LifecycleObservers {
    pub fn new(sweep: ObserverHandle, session_deleted: ObserverHandle) -> Self {
        Self {
            sweep: std::sync::Mutex::new(sweep),
            _session_deleted: session_deleted,
        }
    }
}

/// Observe `cron::on-tick` for the current `sweep_expression`. The observer
/// stays `pending` while the cron worker is away, and a bad expression ends
/// it in `error` (`INVALID_CRON_EXPRESSION`), which is logged.
pub fn observe_sweep(iii: &IIIClient, cfg: &WorkerConfig) -> ObserverHandle {
    let handle = iii.observe(
        Source::function(ON_TICK_FN)
            .args(json!({ "expression": cfg.sweep_expression }))
            .deliver(Deliver::Output),
        SWEEP_PENDING_ID,
    );
    log_failure(&handle, ON_TICK_FN);
    handle
}

pub fn observe_session_deleted(iii: &IIIClient) -> ObserverHandle {
    let handle = iii.observe(
        Source::function(ON_SESSION_DELETED_FN).deliver(Deliver::Output),
        ON_SESSION_DELETED_ID,
    );
    log_failure(&handle, ON_SESSION_DELETED_FN);
    handle
}

/// Warn once if the observer ends in `error` or is disposed; a `pending` one
/// waits quietly for its source.
fn log_failure(handle: &ObserverHandle, source: &'static str) {
    let handle = handle.clone();
    tokio::spawn(async move {
        if let Err(error) = handle.ready().await {
            tracing::warn!(source, observer = %handle.id(), %error, "lifecycle observer failed");
        }
    });
}

/// Keep the freshly created observer, then dispose the old one
/// (observe-new-then-dispose-old: a fail-safe overlap).
fn replace_slot(slot: &std::sync::Mutex<ObserverHandle>, new: ObserverHandle) {
    let old = std::mem::replace(&mut *slot.lock().unwrap_or_else(|p| p.into_inner()), new);
    tokio::spawn(async move {
        if let Err(error) = old.dispose().await {
            tracing::warn!(observer = %old.id(), %error, "disposing the previous sweep observer failed");
        }
    });
}

/// Internal `harness::on-config-change` payload: the `configuration::on-updated`
/// event, delivered as the output. The handler re-fetches the authoritative
/// configuration, so it reads only the (advisory) configuration id; a struct
/// keeps the request schema concrete.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `harness::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and observe
/// `configuration::on-updated` for this worker's entry with it. `handles`
/// holds the live sweep observer the handler replaces when
/// `sweep_expression` changes.
pub fn observe_config_changes(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    handles: Arc<LifecycleObservers>,
) -> ObserverHandle {
    let cell_for_fn = cell.clone();
    let handles_for_fn = handles.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let handles = handles_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &handles).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload harness from the authoritative configuration when it changes — \
             replaces the cron pending-sweep observer on a sweep_expression change and swaps the \
             per-call tuning snapshot otherwise.",
        )
        .metadata(json!({ "internal": true })),
    );

    iii.observe(
        Source::function(ON_CONFIGURATION_UPDATED_FN)
            .filters(config_change_filters(config_id()))
            .deliver(Deliver::Output),
        CONFIG_FN_ID,
    )
}

/// The calls of `configuration::on-updated` this worker reloads on: updates
/// of its own entry.
fn config_change_filters(id: &str) -> Value {
    json!({
        "input.id": { "$eq": id },
        "input.event_type": { "$eq": "configuration:updated" },
    })
}

/// Reload from the AUTHORITATIVE configuration. The delivered event is
/// intentionally ignored: a direct call can never inject config.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, handles: &LifecycleObservers) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let old = cell.read().await.clone();
    if old.boot_signature() != cfg.boot_signature() {
        replace_slot(&handles.sweep, observe_sweep(iii, &cfg));
        tracing::info!("harness pending-sweep observer replaced (sweep_expression changed)");
    }

    apply_config(cell, cfg).await;
    tracing::info!("harness configuration reloaded");
}

async fn call_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .call(
                CallRequest {
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

    /// The missing-entry classifier only seeds on the configuration worker's
    /// standalone `NOT_FOUND` envelope; every unrelated failure or compound
    /// code propagates instead of clobbering a stored value with a default.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(super::is_not_found);
    }

    /// Preserve operator settings when configuration lookup fails for another reason.
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

    use super::*;

    #[test]
    fn is_legacy_harness_value_detects_old_entry_shape() {
        assert!(is_legacy_harness_value(&json!({
            "permissions": { "default_mode": "manual" }
        })));
        assert!(is_legacy_harness_value(&json!({
            "providers": { "anthropic": {} }
        })));
        assert!(!is_legacy_harness_value(&WorkerConfig::default().to_json()));
        assert!(!is_legacy_harness_value(&json!(null)));
    }

    #[test]
    fn drop_provider_identity_prompt_preserves_other_stored_settings() {
        let mut stored = json!({
            "provider_identity_prompt": true,
            "default_max_turns": "${MAX_TURNS:500}",
            "nested": { "provider_identity_prompt": true }
        });

        assert!(drop_provider_identity_prompt(&mut stored));
        assert_eq!(
            stored,
            json!({
                "default_max_turns": "${MAX_TURNS:500}",
                "nested": { "provider_identity_prompt": true }
            })
        );
    }

    #[test]
    fn provider_identity_prompt_migration_preserves_an_intervening_update() {
        let mut expected = json!({ "provider_identity_prompt": true, "turns": 1 });
        let first_write = json!({ "turns": 1 });

        let rebased = reconcile_provider_identity_prompt_write(
            &mut expected,
            &first_write,
            json!({ "provider_identity_prompt": false, "turns": 2 }),
        )
        .expect("the intervening value needs a compensating write");

        assert_eq!(rebased, json!({ "turns": 2 }));
        assert!(
            reconcile_provider_identity_prompt_write(&mut expected, &rebased, first_write,)
                .is_none()
        );
    }

    #[test]
    fn configuration_get_payload_keeps_raw_reads_separate_from_expanded_reads() {
        assert_eq!(
            configuration_get_payload(true),
            json!({ "id": config_id(), "raw": true })
        );
        assert_eq!(
            configuration_get_payload(false),
            json!({ "id": config_id(), "raw": false })
        );
    }

    #[test]
    fn config_changes_select_updates_of_this_entry_only() {
        assert_eq!(
            config_change_filters("harness"),
            json!({
                "input.id": { "$eq": "harness" },
                "input.event_type": { "$eq": "configuration:updated" },
            })
        );
    }

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.default_max_turns, 500);
        apply_config(
            &cell,
            WorkerConfig {
                default_max_turns: 99,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.default_max_turns, 99);
    }
}
