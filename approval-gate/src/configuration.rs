//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors [`context-manager`](../../context-manager/src/configuration.rs) /
//! [`session-manager`](../../session-manager/src/configuration.rs).
//!
//! Every configuration field hot-reloads via a snapshot swap — nothing
//! requires a restart. The observer of the harness's
//! `harness::hook::on-pre-call` is fixed at worker startup (consult on all
//! calls, fail closed).
//!
//! `configuration` is a REQUIRED boot dependency: a failed register/fetch
//! aborts startup (the gate must run on a known, authoritative policy
//! surface, never a guessed one).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{
    CallRequest, Deliver, IIIClient, ObserverHandle, ObserverState, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a
/// `read().await` and `clone()` the inner `Arc` out (a cheap refcount
/// bump) without holding the lock across its work, while `apply_config`
/// whole-snapshot replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const DEFAULT_CONFIG_ID: &str = "approval-gate";

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
const CONFIG_FN_ID: &str = "approval::on-config-change";
const CONFIG_FN_DESC: &str = "Internal: hot-reload approval-gate from the authoritative \
     configuration when it changes — swaps the per-call snapshot (default_mode, rules, \
     grant_reask_limit).";
/// The configuration worker's change event, observed by
/// [`observe_config_changes`].
const CONFIG_ON_UPDATED_FN: &str = "configuration::on-updated";
/// The harness's hook functions this worker observes.
pub const HOOK_ON_PRE_CALL_FN: &str = "harness::hook::on-pre-call";
pub const HOOK_ON_POST_CALL_FN: &str = "harness::hook::on-post-call";
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the
/// attempt number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Fixed args of the `harness::hook::on-pre-call` observer — approval-gate
/// consults on every function call.
const HOOK_FUNCTIONS: &[&str] = &["*"];
const HOOK_TIMEOUT_MS: u64 = 5_000;
const HOOK_ON_ERROR: &str = "fail_closed";

/// Fixed args of the `harness::hook::on-post-call` observer for
/// `approval::filesystem-access-watch` — only `shell::*` / `coder::*`
/// dispatch results carry a `filesystem_access_request` worth watching for.
/// `fail_open`, where the pre-call gate fails closed: a crashed/timed-out
/// filesystem-access-watch must never turn an already-decided
/// function_result into a stuck call.
const FILESYSTEM_ACCESS_WATCH_FUNCTIONS: &[&str] = &["shell::*", "coder::*"];
const FILESYSTEM_ACCESS_WATCH_TIMEOUT_MS: u64 = 5_000;
const FILESYSTEM_ACCESS_WATCH_ON_ERROR: &str = "fail_open";

// Routed calls include engine metadata such as `_caller_worker_id`.
// The payload never controls which configuration entry is returned.
#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ConfigurationIdentityRequest {}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct ConfigurationIdentityResponse {
    id: String,
}

/// Register the schema, seeding only when no stored value exists. Publish
/// the instance's identity in its namespace so UI policy edits cannot target
/// another project's configuration.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    iii.register_function(
        "approval-gate::configuration-id",
        RegisterFunction::new(|_request: ConfigurationIdentityRequest| {
            Ok::<_, Error>(ConfigurationIdentityResponse {
                id: config_id().to_string(),
            })
        })
        .description(
            "Returns this approval-gate instance's configuration entry ID, without its value.",
        )
        .metadata(json!({ "internal": true })),
    );
    let mut payload = json!({
        "id": config_id(),
        "name": "Approval Gate",
        "description": "Policy and decision surface settings: the deployment \
                        approval defaults (permission mode for new sessions \
                        and the auto-mode trust seed) and the agent permission \
                        rules.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = seed.cloned().unwrap_or_default().to_json();
    ensure_configuration(iii, payload).await
}

/// Read the live `approval-gate` configuration (env-expanded by the
/// configuration worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        call_configuration_with_retry(iii, function, payload)
    })
    .await
}

#[path = "../../crates/config-client/src/initialization.rs"]
mod initialization;

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// Returns `Ok(None)` only for a missing entry. A missing service
/// (`function_not_found`) must propagate rather than authorize seeding.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match call_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Observe `harness::hook::on-pre-call` with `approval::gate` at worker
/// startup. The observer stays pending while the harness is not running and
/// becomes active once the harness registers its hook function, so start
/// order does not matter.
pub fn observe_hook(iii: &IIIClient) -> ObserverHandle {
    observe_hook_point(
        iii,
        hook_source(
            HOOK_ON_PRE_CALL_FN,
            HOOK_FUNCTIONS,
            HOOK_TIMEOUT_MS,
            HOOK_ON_ERROR,
        ),
        crate::functions::GATE_ID,
    )
}

/// Observe `harness::hook::on-post-call` with
/// `approval::filesystem-access-watch` at worker startup — beside
/// [`observe_hook`], pending in the same way while the harness is absent.
pub fn observe_filesystem_access_watch_hook(iii: &IIIClient) -> ObserverHandle {
    observe_hook_point(
        iii,
        hook_source(
            HOOK_ON_POST_CALL_FN,
            FILESYSTEM_ACCESS_WATCH_FUNCTIONS,
            FILESYSTEM_ACCESS_WATCH_TIMEOUT_MS,
            FILESYSTEM_ACCESS_WATCH_ON_ERROR,
        ),
        crate::functions::FILESYSTEM_ACCESS_WATCH_ID,
    )
}

/// The harness hook function with this observer's args: the called function
/// globs it consults on, the per-call timeout and the failure policy.
fn hook_source(function_id: &str, functions: &[&str], timeout_ms: u64, on_error: &str) -> Source {
    Source::function(function_id).args(json!({
        "functions": functions,
        "timeout_ms": timeout_ms,
        "on_error": on_error,
    }))
}

fn observe_hook_point(iii: &IIIClient, source: Source, target: &'static str) -> ObserverHandle {
    let hook = source.function_id().to_string();
    let handle = iii.observe(source, target);
    let _ = handle.on_state_change(move |state, reason| match state {
        ObserverState::Active => {
            tracing::info!(hook = %hook, function_id = target, "hook observer active")
        }
        ObserverState::Error => tracing::warn!(
            hook = %hook,
            function_id = target,
            reason = ?reason,
            "the harness refused the hook observer"
        ),
        _ => {}
    });
    handle
}

/// The `configuration::on-updated` event, delivered as output to the
/// internal `approval::on-config-change`. The handler re-fetches the
/// authoritative configuration, so only the (advisory) configuration id is
/// read; a struct (not `Value`) keeps the request schema concrete and unknown
/// fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `approval::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and observe
/// `configuration::on-updated` with it: this entry's `configuration:updated`
/// events, delivered as output. The handler re-fetches via
/// `configuration::get` and ignores the event, so a direct call can never
/// inject config.
pub fn observe_config_changes(iii: &IIIClient, cell: ConfigCell) -> ObserverHandle {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(CONFIG_FN_DESC)
        .metadata(json!({ "internal": true })),
    );

    iii.observe(
        Source::function(CONFIG_ON_UPDATED_FN)
            .filters(config_update_filters(config_id()))
            .deliver(Deliver::Output),
        CONFIG_FN_ID,
    )
}

/// Selects the `configuration:updated` events of one entry.
fn config_update_filters(config_id: &str) -> Value {
    json!({
        "input.id": { "$eq": config_id },
        "input.event_type": { "$eq": "configuration:updated" },
    })
}

/// Reload from the AUTHORITATIVE configuration.
///
/// The delivered event is intentionally ignored:
/// `approval::on-config-change` is a discoverable bus function, so
/// trusting `payload.new_value` would let any caller inject arbitrary
/// config without updating persisted state. Re-fetch the stored value via
/// `configuration::get` instead.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous config"
            );
            return;
        }
    };

    apply_config(cell, cfg).await;
    tracing::info!("approval-gate configuration reloaded");
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
                    timeout_ms: None,
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
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_err,
                        "configuration RPC failed; retrying"
                    );
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

    /// Engine-injected caller metadata must not prevent resolving this worker's entry ID.
    #[test]
    fn configuration_identity_accepts_engine_caller_metadata() {
        for payload in [
            serde_json::json!({}),
            serde_json::json!({
                "_caller_worker_id": "00000000-0000-4000-8000-000000000002"
            }),
        ] {
            serde_json::from_value::<super::ConfigurationIdentityRequest>(payload)
                .expect("routed identity requests accept engine metadata");
        }
    }

    use super::*;
    use crate::types::PermissionMode;

    #[test]
    fn hooks_observe_the_harness_hook_functions_with_fixed_args() {
        assert_eq!(
            hook_source(
                HOOK_ON_PRE_CALL_FN,
                HOOK_FUNCTIONS,
                HOOK_TIMEOUT_MS,
                HOOK_ON_ERROR
            ),
            Source::function("harness::hook::on-pre-call").args(serde_json::json!({
                "functions": ["*"],
                "timeout_ms": 5_000,
                "on_error": "fail_closed",
            }))
        );
        assert_eq!(
            hook_source(
                HOOK_ON_POST_CALL_FN,
                FILESYSTEM_ACCESS_WATCH_FUNCTIONS,
                FILESYSTEM_ACCESS_WATCH_TIMEOUT_MS,
                FILESYSTEM_ACCESS_WATCH_ON_ERROR
            ),
            Source::function("harness::hook::on-post-call").args(serde_json::json!({
                "functions": ["shell::*", "coder::*"],
                "timeout_ms": 5_000,
                "on_error": "fail_open",
            }))
        );
    }

    #[test]
    fn config_changes_select_one_entrys_updates() {
        assert_eq!(
            config_update_filters("approval-gate"),
            serde_json::json!({
                "input.id": { "$eq": "approval-gate" },
                "input.event_type": { "$eq": "configuration:updated" },
            })
        );
    }

    /// The hot-reload description names every field the snapshot swap
    /// replaces: each top-level property of the configuration schema.
    #[test]
    fn config_change_description_names_every_config_field() {
        let schema = WorkerConfig::json_schema();
        let fields = schema["properties"]
            .as_object()
            .expect("schema has a properties object");
        assert!(!fields.is_empty());
        for field in fields.keys() {
            assert!(
                CONFIG_FN_DESC.contains(field.as_str()),
                "approval::on-config-change's description does not name `{field}`"
            );
        }
    }

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.default_mode, PermissionMode::Manual);

        let tuned = WorkerConfig {
            default_mode: PermissionMode::Full,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.default_mode, PermissionMode::Full);
    }
}
