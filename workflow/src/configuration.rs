//! Integration with the builtin `configuration` worker (plumbing shared via
//! `crates/config-client`) — register the schema, fetch the authoritative
//! value at boot, and hot-reload it when it changes.
//!
//! `sweep_expression` is the one STRUCTURAL field (the cron binding for the
//! node-timeout sweep). On a change the handler re-binds the trigger live
//! (register-new-then-unregister-old: a fail-safe overlap).
//!
//! Every other field is a per-call tuning knob read from the live snapshot via
//! [`Deps::cfg`](crate::functions::Deps::cfg); a change swaps the snapshot.

use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::trigger::Trigger;
use iii_sdk::IIIClient;
use serde_json::json;

use crate::config::WorkerConfig;
// Reuse the ConfigCell type declared in functions::mod — do NOT redefine.
use crate::functions::ConfigCell;

pub const DEFAULT_CONFIG_ID: &str = "workflow";

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
pub const CONFIG_FN_ID: &str = "workflow::on-config-change";
pub const SWEEP_ID: &str = "workflow::sweep";

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: config_id(),
        name: "Workflow",
        description: "Workflow worker settings: default node-pending timeout, \
                      cron sweep schedule, RPC dispatch timeout, and max node retries.",
        schema: WorkerConfig::json_schema(),
        default_value: WorkerConfig::default().to_json(),
    }
}

/// Register the `workflow` configuration schema. The built-in default is
/// seeded as `initial_value` only when nothing is stored yet (safe to call
/// every boot).
pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    config_client::register(iii, &spec(), None).await
}

/// Read the live `workflow` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    match config_client::fetch(iii, config_id()).await? {
        Some(v) => WorkerConfig::from_json(&v),
        None => {
            tracing::info!("no configuration value found; using built-in default configuration");
            Ok(WorkerConfig::default())
        }
    }
}

/// Swap the config snapshot under the write lock.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    // Keep the state-layer RPC timeout in sync with the live config.
    crate::state::set_dispatch_timeout_ms(cfg.dispatch_timeout_ms);
    *cell.write().await = Arc::new(cfg);
}

/// Live handles for the hot-reloadable trigger bindings: the cron sweep and
/// the pre-generate guidance hook (bound only while `inject_guidance` is on).
pub struct TriggerHandles {
    pub sweep: std::sync::Mutex<Option<Trigger>>,
    pub guidance: config_client::BindingSlot,
}

/// Bind the two always-on harness hooks (turn-completed → wake, pre-trigger →
/// stamp-reply). One shot: if the harness is not up yet, the engine parks each
/// binding as a pending intent and activates it when the trigger type registers
/// (recoverable triggers, iii #1962) — and re-parks/re-activates them across
/// harness restarts. Nothing to watch or retry. The pre-generate guidance hook
/// is NOT bound here — it follows the `inject_guidance` config knob via
/// [`apply_guidance`].
pub fn setup_harness_hooks(iii: &IIIClient) {
    let _ = bind_turn_completed(iii);
    let _ = bind_pre_trigger_hook(iii);
    tracing::info!(
        "workflow harness hooks registered: turn-completed → wake, pre-trigger → \
         stamp-reply"
    );
}

/// Reconcile the live `workflow::inject-guidance` binding with the configured
/// `inject_guidance` value: on → bind once; off → unregister and drop the
/// handle. Idempotent under repeated config events, and a failed bind retries
/// on the next event.
pub fn apply_guidance(iii: &IIIClient, handles: &TriggerHandles, enabled: bool) {
    handles.guidance.reconcile(
        enabled,
        || bind_pre_generate_hook(iii),
        "inject_guidance on: appending workflow guidance to agent system prompts",
        "inject_guidance off: workflow guidance stays out of agent system prompts",
    );
}

/// (Re)bind the cron node-timeout sweep from the current config. Best-effort
/// (the cron trigger type always exists, but a transient failure must not
/// brick boot): a failure surfaces as a `None` handle.
pub fn bind_sweep(iii: &IIIClient, cfg: &WorkerConfig) -> Option<Trigger> {
    config_client::try_bind(
        iii,
        RegisterTriggerInput {
            trigger_type: "cron".to_string(),
            function_id: SWEEP_ID.to_string(),
            config: json!({ "expression": cfg.sweep_expression }),
            metadata: None,
        },
    )
}

/// Subscribe `workflow::wake` to harness turn-completion. No session filter —
/// the handler discards non-workflow sessions — because workflow nodes are
/// top-level turns, so a `parent_session_id` filter would never match. The cron
/// sweep is the durable fallback if this best-effort bind fails (harness not up).
fn bind_turn_completed(iii: &IIIClient) -> Option<Trigger> {
    config_client::try_bind(
        iii,
        RegisterTriggerInput {
            trigger_type: "harness::turn-completed".to_string(),
            function_id: crate::functions::wake::WAKE_ID.to_string(),
            config: json!({}),
            metadata: None,
        },
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
    config_client::try_bind(
        iii,
        RegisterTriggerInput {
            trigger_type: "harness::hook::pre-trigger".to_string(),
            function_id: crate::functions::stamp_reply::STAMP_REPLY_ID.to_string(),
            config: json!({ "functions": ["workflow::start"], "on_error": "fail_open", "timeout_ms": 30000 }),
            metadata: None,
        },
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
    config_client::try_bind(
        iii,
        RegisterTriggerInput {
            trigger_type: "harness::hook::pre-generate".to_string(),
            function_id: crate::functions::inject_guidance::GUIDANCE_HOOK_ID.to_string(),
            config: json!({ "on_error": "fail_open" }),
            metadata: Some(json!({
                "inject_prompt": crate::functions::inject_guidance::WORKFLOW_GUIDANCE
            })),
        },
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

/// Register the internal `workflow::on-config-change` handler and bind a
/// `configuration` trigger. `handles` holds the live cron `Trigger` the
/// handler re-binds when `sweep_expression` changes. Every delivery runs the
/// reload under the shared lock (fetch inside it); the returned
/// [`config_client::Reload`] lets boot run one extra pass to close the
/// fetch→bind gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    handles: Arc<TriggerHandles>,
) -> Result<config_client::Reload, Error> {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        config_id(),
        CONFIG_FN_ID,
        "Internal: hot-reload workflow config — re-binds the cron sweep on a \
         sweep_expression change and swaps the per-call tuning snapshot otherwise.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            let handles = handles.clone();
            async move { on_config_change(&engine, &cell, &handles).await }
        },
    )
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

    let inject = applied.inject_guidance;
    apply_config(cell, applied).await;
    apply_guidance(iii, handles, inject);
    tracing::info!(inject_guidance = inject, "workflow configuration reloaded");
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
}
