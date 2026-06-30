//! Integration with the `configuration` worker — register the schema, fetch
//! the authoritative value at boot, and hot-reload it when it changes.
//!
//! `sweep_expression` is the one STRUCTURAL field (the cron binding for the
//! node-timeout sweep). On a change the handler re-binds the trigger live
//! (register-new-then-unregister-old: a fail-safe overlap).
//!
//! Every other field is a per-call tuning knob read from the live snapshot via
//! [`Deps::cfg`](crate::functions::Deps::cfg); a change swaps the snapshot.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::WorkerConfig;
// Reuse the ConfigCell type declared in functions::mod — do NOT redefine.
use crate::functions::ConfigCell;

pub const CONFIG_ID: &str = "workflow";
const CONFIG_FN_ID: &str = "workflow::on-config-change";
pub const SWEEP_ID: &str = "workflow::sweep";

const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the `workflow` configuration schema. The built-in default is seeded
/// as `initial_value` only when nothing is stored yet (safe to call every boot).
pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Workflow",
        "description": "Workflow worker settings: default node-pending timeout, \
                        cron sweep schedule, RPC dispatch timeout, and max node retries.",
        "schema": WorkerConfig::json_schema(),
    });
    if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `workflow` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

/// Returns `Ok(None)` when the entry does not exist.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    // Keep the state-layer RPC timeout in sync with the live config.
    crate::state::set_dispatch_timeout_ms(cfg.dispatch_timeout_ms);
    *cell.write().await = Arc::new(cfg);
}

/// Live handle for the one hot-reloadable trigger binding — the cron sweep.
pub struct TriggerHandles {
    pub sweep: std::sync::Mutex<Option<Trigger>>,
}

/// Best-effort binding: the cron trigger type always exists (engine built-in),
/// but a transient failure must not brick boot — it surfaces as a `None` handle.
fn bind(iii: &IIIClient, trigger_type: &str, function_id: &str, config: Value) -> Option<Trigger> {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    }) {
        Ok(handle) => {
            tracing::info!(trigger_type, function_id, "trigger binding requested");
            Some(handle)
        }
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed");
            None
        }
    }
}

/// Harness-PROVIDED trigger types the workflow worker binds to (NOT engine
/// built-ins like `cron`). On a cold start the workflow worker can boot before the
/// harness has registered these; since `register_trigger` is fire-and-forget (the
/// engine async-rejects a bind to an unknown trigger type and silently drops it),
/// binding too early loses the wake / reply-stamp / guidance hooks, so we bind only
/// once these are present (driven by `setup_harness_hooks`).
const HARNESS_TRIGGER_TYPES: [&str; 3] = [
    "harness::turn-completed",
    "harness::hook::pre-trigger",
    "harness::hook::pre-generate",
];

/// True iff every id in `needed` appears as a trigger-type `id` in an
/// `engine::triggers::list` response. Pure, so it's unit-testable.
fn all_trigger_types_present(resp: &Value, needed: &[&str]) -> bool {
    let Some(types) = resp.get("triggers").and_then(|v| v.as_array()) else {
        return false;
    };
    needed.iter().all(|n| {
        types
            .iter()
            .any(|t| t.get("id").and_then(|v| v.as_str()) == Some(*n))
    })
}

/// One probe: are all harness trigger types registered on the engine right now?
async fn harness_trigger_types_ready(iii: &IIIClient) -> bool {
    match iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::list".into(),
            payload: json!({ "include_internal": true }),
            action: None,
            timeout_ms: Some(CONFIG_TIMEOUT_MS),
        })
        .await
    {
        Ok(resp) => all_trigger_types_present(&resp, &HARNESS_TRIGGER_TYPES),
        Err(_) => false, // engine not reachable / harness not up yet
    }
}

const ON_REGISTRY_CHANGED_FN_ID: &str = "workflow::on-registry-changed";
/// Two mandatory in-process engine triggers we subscribe to so we bind the harness hooks
/// as soon as the harness is up. Binding to them never races (both registered at engine
/// boot). They cover DIFFERENT timings, and the harness needs the second one:
///   - `engine::workers-available` fires IMMEDIATELY on any worker connect / metadata /
///     disconnect. But the SDK auto-sends a worker's metadata register right after connect,
///     which can PRECEDE that worker registering its own trigger types: the harness
///     registers `harness::hook::*` only after two awaited config RPCs (harness main.rs),
///     so its workers-available firings land BEFORE its types exist — a probe then misses.
///   - `engine::functions-available` fires via the engine's internal function-set poll,
///     AFTER a worker registers its functions. The harness registers its trigger types
///     before its functions, so this firing reliably post-dates the harness's types. This
///     is the event that closes the cold-start hole workers-available alone left open.
const WORKERS_AVAILABLE_TRIGGER_TYPE: &str = "engine::workers-available";
const FUNCTIONS_AVAILABLE_TRIGGER_TYPE: &str = "engine::functions-available";

/// Bind the three harness hooks (turn-completed → wake, pre-trigger → stamp-reply,
/// pre-generate → inject-guidance) EXACTLY ONCE, and only after the harness has
/// registered their trigger types (one `engine::triggers::list` probe). Because
/// `register_trigger` is fire-and-forget — the engine silently drops a bind to an
/// unknown trigger type — binding before the harness is up would lose the hooks. The
/// shared `bound` flag keeps this idempotent across the boot-time attempt and every
/// registry-change firing (which may run concurrently). After the first success the
/// engine re-binds these automatically if the harness later restarts
/// (`register_trigger_type` re-scans stored triggers), so one pass is enough.
async fn try_bind_harness_hooks(iii: &IIIClient, bound: &AtomicBool) {
    if bound.load(Ordering::Acquire) {
        return;
    }
    if !harness_trigger_types_ready(iii).await {
        return; // harness not up yet; a later registry-change event retries
    }
    // Win the bind race exactly once even if several worker events fire together.
    if bound
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let turn_completed = bind_turn_completed(iii).is_some();
    let pre_trigger = bind_pre_trigger_hook(iii).is_some();
    let pre_generate = bind_pre_generate_hook(iii).is_some();
    if turn_completed && pre_trigger && pre_generate {
        tracing::info!(
            "workflow harness hooks registered: turn-completed → wake, pre-trigger → \
             stamp-reply, pre-generate → inject-guidance (guidance injection active)"
        );
    } else {
        // A bind transiently failed. Release the claim so a later engine::*-available
        // event retries, instead of leaving the missing hook(s) unbound until restart.
        // Re-binding an already-bound hook on retry is tolerated: the bound hooks
        // (turn-completed → wake, pre-trigger → stamp) are idempotent downstream.
        bound.store(false, Ordering::Release);
        tracing::warn!(
            turn_completed,
            pre_trigger,
            pre_generate,
            "not all workflow harness hooks bound; releasing the bind claim to retry on the \
             next registry-change event"
        );
    }
}

/// Bind the harness hooks without racing the harness's startup — and without polling.
///
/// The workflow worker can boot before the harness has registered its hook trigger types.
/// Rather than poll, we subscribe to two engine registry-change triggers and (re)try the
/// binds on each firing; `try_bind_harness_hooks` no-ops until the harness's types appear,
/// then binds once. We need BOTH because they fire at different moments (see the trigger
/// consts): `engine::workers-available` is immediate but can precede a worker's own type
/// registration, while `engine::functions-available` fires after function registration and
/// so reliably post-dates the harness's types. The initial attempt covers the warm-start
/// case (harness already connected, so no new event fires for it).
pub async fn setup_harness_hooks(iii: &Arc<IIIClient>) {
    let bound = Arc::new(AtomicBool::new(false));

    let iii_for_fn = iii.clone();
    let bound_for_fn = bound.clone();
    iii.register_function(
        ON_REGISTRY_CHANGED_FN_ID,
        RegisterFunction::new_async(move |_event: Value| {
            let iii = iii_for_fn.clone();
            let bound = bound_for_fn.clone();
            async move {
                try_bind_harness_hooks(&iii, &bound).await;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        })
        .description(
            "Internal: on engine::workers-available / engine::functions-available, bind the \
             workflow harness hooks (turn-completed, pre-trigger, pre-generate) once the \
             harness has registered their trigger types. Not called directly.",
        ),
    );

    // Arm the event-driven retries BEFORE the initial probe, so a harness that comes up
    // mid-probe is still caught. Both binds are race-free (mandatory in-process types).
    for trigger_type in [
        WORKERS_AVAILABLE_TRIGGER_TYPE,
        FUNCTIONS_AVAILABLE_TRIGGER_TYPE,
    ] {
        let _ = bind(iii, trigger_type, ON_REGISTRY_CHANGED_FN_ID, json!({}));
    }

    // Warm start: the harness may already be connected, in which case no further
    // registry-change event fires for it — attempt the bind now.
    try_bind_harness_hooks(iii, &bound).await;
}

/// (Re)bind the cron node-timeout sweep from the current config.
pub fn bind_sweep(iii: &IIIClient, cfg: &WorkerConfig) -> Option<Trigger> {
    bind(
        iii,
        "cron",
        SWEEP_ID,
        json!({ "expression": cfg.sweep_expression }),
    )
}

/// Subscribe `workflow::wake` to harness turn-completion. No session filter —
/// the handler discards non-workflow sessions — because workflow nodes are
/// top-level turns, so a `parent_session_id` filter would never match. The cron
/// sweep is the durable fallback if this best-effort bind fails (harness not up).
fn bind_turn_completed(iii: &IIIClient) -> Option<Trigger> {
    bind(
        iii,
        "harness::turn-completed",
        crate::functions::wake::WAKE_ID,
        json!({}),
    )
}

/// Bind the `workflow::stamp-reply` pre_trigger hook to `workflow::start`. It only
/// stamps the caller's session (and, when `reply_to` is set, the caller's
/// model/provider/policy) into the arguments — trivial, fast, and side-effect-free;
/// it never starts a run or parks the call.
///
/// `on_error: fail_open` because of that: a hook timeout/error must NOT block a
/// legitimate `workflow::start`. Falling through just runs the call without the
/// auto-stamp (the run still happens; it only loses console nesting / reply
/// delivery on that one call) — strictly better than denying it.
fn bind_pre_trigger_hook(iii: &IIIClient) -> Option<Trigger> {
    bind(
        iii,
        "harness::hook::pre-trigger",
        crate::functions::stamp_reply::STAMP_REPLY_ID,
        json!({ "functions": ["workflow::start"], "on_error": "fail_open", "timeout_ms": 30000 }),
    )
}

/// Bind the `workflow::inject-guidance` pre_generate hook so the workflow
/// orchestration guidance is appended to the agent system prompt ONLY while this
/// worker is connected (the binding is dropped when the worker goes away). No
/// `functions` filter — pre_generate is not function-scoped. `on_error: fail_open`
/// is MANDATORY: pre_generate defaults to fail-CLOSED, which would abort generation
/// if this hook ever errored or timed out; a missing guidance line must never block
/// a turn. Best-effort bind like the others.
fn bind_pre_generate_hook(iii: &IIIClient) -> Option<Trigger> {
    bind(
        iii,
        "harness::hook::pre-generate",
        crate::functions::inject_guidance::GUIDANCE_HOOK_ID,
        json!({ "on_error": "fail_open" }),
    )
}

/// Store the freshly-registered handle, then unregister the old one
/// (register-new-then-unregister-old: a fail-safe overlap).
fn rebind_slot(slot: &std::sync::Mutex<Option<Trigger>>, new: Option<Trigger>) {
    let Some(new) = new else {
        return;
    };
    let old = slot.lock().unwrap_or_else(|p| p.into_inner()).replace(new);
    if let Some(old) = old {
        old.unregister();
    }
}

/// Internal `workflow::on-config-change` trigger payload.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `workflow::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. `handles` holds the live cron `Trigger` the handler re-binds when
/// `sweep_expression` changes.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    handles: Arc<TriggerHandles>,
) -> Result<(), Error> {
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
            "Internal: hot-reload workflow config — re-binds the cron sweep on a \
             sweep_expression change and swaps the per-call tuning snapshot otherwise.",
        ),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
    })?;
    Ok(())
}

/// Reload from the AUTHORITATIVE configuration. The caller-supplied trigger
/// payload is intentionally ignored: a direct call can never inject config.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, handles: &TriggerHandles) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let old = cell.read().await.clone();
    // Re-bind the cron sweep only when a boot-relevant field changes — one baked
    // into a live trigger binding rather than hot-applied per call. Today that is
    // only the sweep schedule. Commit the new expression into the live snapshot
    // ONLY when the rebind actually succeeds; on failure keep the old expression so
    // the next config-change still sees a diff and retries — otherwise the snapshot
    // would advertise a schedule the active cron trigger isn't running.
    let mut applied = cfg;
    if old.sweep_expression != applied.sweep_expression {
        match bind_sweep(iii, &applied) {
            Some(trigger) => {
                rebind_slot(&handles.sweep, Some(trigger));
                tracing::info!("workflow sweep re-bound (sweep_expression changed)");
            }
            None => {
                applied.sweep_expression = old.sweep_expression.clone();
                tracing::error!(
                    "workflow sweep rebind failed; keeping the previous sweep binding and schedule"
                );
            }
        }
    }

    apply_config(cell, applied).await;
    tracing::info!("workflow configuration reloaded");
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
    use super::*;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.default_pending_timeout_ms, 1_800_000);
        apply_config(
            &cell,
            WorkerConfig {
                default_pending_timeout_ms: 9999,
                ..WorkerConfig::default()
            },
        )
        .await;
        assert_eq!(cell.read().await.default_pending_timeout_ms, 9999);
    }

    #[test]
    fn all_trigger_types_present_requires_every_needed_id() {
        // All three harness trigger types present → ready.
        let ready = serde_json::json!({
            "triggers": [
                { "id": "harness::turn-completed" },
                { "id": "harness::hook::pre-trigger" },
                { "id": "harness::hook::pre-generate" },
                { "id": "cron" },
            ]
        });
        assert!(all_trigger_types_present(&ready, &HARNESS_TRIGGER_TYPES));

        // One missing (the exact cold-start bug: harness up but pre-generate not
        // yet registered) → NOT ready, so we keep waiting instead of binding early.
        let partial = serde_json::json!({
            "triggers": [
                { "id": "harness::turn-completed" },
                { "id": "harness::hook::pre-trigger" },
            ]
        });
        assert!(!all_trigger_types_present(&partial, &HARNESS_TRIGGER_TYPES));

        // No triggers field at all (engine unreachable / empty) → NOT ready.
        assert!(!all_trigger_types_present(
            &serde_json::json!({}),
            &HARNESS_TRIGGER_TYPES
        ));
    }
}
