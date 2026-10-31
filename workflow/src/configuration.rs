//! Integration with the builtin `configuration` worker (plumbing shared via
//! `crates/config-client`) — register the schema, fetch the authoritative
//! value at boot, and hot-reload it when it changes.
//!
//! `sweep_expression` is the one STRUCTURAL field (the args of the
//! `cron::on-tick` observer that runs the node-timeout sweep). On a change the
//! handler observes the new expression, then disposes the old observer
//! (observe-new-then-dispose-old: a fail-safe overlap).
//!
//! Every other field is a per-call tuning knob read from the live snapshot via
//! [`Deps::cfg`](crate::functions::Deps::cfg); a change swaps the snapshot.

use std::sync::Arc;
use std::time::Duration;

use iii_config_client as config_client;
use iii_sdk::{Deliver, IIIClient, ObserverHandle, Source};
use serde_json::{json, Value};

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

/// The cron worker's tick; the sweep observes it with args `{ expression }`.
pub const CRON_ON_TICK_FN: &str = "cron::on-tick";
/// The harness's turn completion; `workflow::wake` observes it.
pub const TURN_COMPLETED_FN: &str = "harness::on-turn-completed";
/// The harness hook point around a function call; `workflow::stamp-reply`
/// observes it scoped to `workflow::start`.
pub const PRE_CALL_HOOK_FN: &str = "harness::hook::on-pre-call";
/// The harness hook point before generation; the guidance observer.
pub const PRE_GENERATE_HOOK_FN: &str = "harness::hook::on-pre-generate";

/// How long a replacement sweep observer may stay `pending` before it is
/// kept anyway (the cron worker is not running yet).
const SWEEP_READY_WAIT: Duration = Duration::from_secs(10);

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: config_id(),
        form_id: DEFAULT_CONFIG_ID,
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
    config_client::ensure(iii, &spec(), None).await
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

/// Live observer handles for the hot-reloadable observers: the cron sweep and
/// the pre-generate guidance hook (observed only while `inject_guidance` is on).
pub struct ObserverHandles {
    pub sweep: std::sync::Mutex<ObserverHandle>,
    pub guidance: config_client::ObserverSlot,
}

/// Observe the two always-on harness functions (turn-completed → wake,
/// pre-call hook → stamp-reply). One shot: an observer of a function the
/// harness has not registered yet stays `pending` and becomes active when the
/// harness registers it, and again across harness restarts. Nothing to watch
/// or retry. The pre-generate guidance hook is NOT observed here — it follows
/// the `inject_guidance` config knob via [`apply_guidance`].
pub fn setup_harness_hooks(iii: &IIIClient) {
    observe_turn_completed(iii);
    observe_pre_call_hook(iii);
    tracing::info!(
        "workflow harness observers created: on-turn-completed → wake, hook::on-pre-call → \
         stamp-reply"
    );
}

/// Reconcile the live `workflow::inject-guidance` observer with the configured
/// `inject_guidance` value: on → observe once; off → dispose the observer and
/// drop the handle. Idempotent under repeated config events.
pub fn apply_guidance(iii: &IIIClient, handles: &ObserverHandles, enabled: bool) {
    handles.guidance.reconcile(
        enabled,
        || observe_pre_generate_hook(iii),
        "inject_guidance on: appending workflow guidance to agent system prompts",
        "inject_guidance off: workflow guidance stays out of agent system prompts",
    );
}

/// The args of the sweep's `cron::on-tick` observer.
pub fn sweep_args(cfg: &WorkerConfig) -> Value {
    json!({ "expression": cfg.sweep_expression })
}

/// Observe `cron::on-tick` with the configured sweep expression; each tick
/// runs `workflow::sweep` with the tick (delivered as output). The observer
/// stays `pending` while the cron worker is absent, and ends in `error`
/// (`INVALID_CRON_EXPRESSION`) when the cron worker refuses the expression.
pub fn observe_sweep(iii: &IIIClient, cfg: &WorkerConfig) -> ObserverHandle {
    iii.observe(
        Source::function(CRON_ON_TICK_FN)
            .args(sweep_args(cfg))
            .deliver(Deliver::Output),
        SWEEP_ID,
    )
}

/// Log once if the boot sweep observer is refused; the node-timeout sweep
/// then waits for a config change with a valid expression.
pub fn report_sweep(handle: ObserverHandle) {
    tokio::spawn(async move {
        if let Ok(Err(e)) = tokio::time::timeout(SWEEP_READY_WAIT, handle.ready()).await {
            tracing::error!(error = %e, "workflow sweep observer refused; fix sweep_expression");
        }
    });
}

/// Subscribe `workflow::wake` to harness turn completion. No session filter —
/// the handler discards non-workflow sessions — because workflow nodes are
/// top-level turns, so a `parent_session_id` filter would never match. The cron
/// sweep is the durable fallback while the harness is down.
fn observe_turn_completed(iii: &IIIClient) -> ObserverHandle {
    iii.observe(
        Source::function(TURN_COMPLETED_FN).deliver(Deliver::Output),
        crate::functions::wake::WAKE_ID,
    )
}

/// The args of the `workflow::stamp-reply` observer of the pre-call hook.
///
/// `on_error: fail_open` because the hook only stamps the caller's session
/// (and, when `reply_to` is set, the caller's model/provider/policy) into the
/// arguments — trivial, fast, and side-effect-free; it never starts a run or
/// parks the call. A hook timeout/error must NOT block a legitimate
/// `workflow::start`: falling through just runs the call without the
/// auto-stamp (the run still happens; it only loses console nesting / reply
/// delivery on that one call) — strictly better than denying it.
pub fn pre_call_hook_args() -> Value {
    json!({ "functions": ["workflow::start"], "on_error": "fail_open", "timeout_ms": 30000 })
}

/// Observe `harness::hook::on-pre-call` with `workflow::stamp-reply`, scoped
/// to `workflow::start`. The harness calls the target with the hook input and
/// applies its decision.
fn observe_pre_call_hook(iii: &IIIClient) -> ObserverHandle {
    iii.observe(
        Source::function(PRE_CALL_HOOK_FN).args(pre_call_hook_args()),
        crate::functions::stamp_reply::STAMP_REPLY_ID,
    )
}

/// The args of the guidance observer of the pre-generate hook. No `functions`
/// filter — pre_generate is not function-scoped. `on_error: fail_open` is
/// MANDATORY: pre_generate defaults to fail-CLOSED, which would abort
/// generation if this hook ever errored or timed out; a missing guidance line
/// must never block a turn. `inject_prompt` is the static contribution the
/// harness joins into the system prompt.
pub fn pre_generate_hook_args() -> Value {
    json!({
        "on_error": "fail_open",
        "inject_prompt": crate::functions::inject_guidance::WORKFLOW_GUIDANCE,
    })
}

/// Observe `harness::hook::on-pre-generate` so the workflow orchestration
/// guidance is appended to the agent system prompt ONLY while this worker is
/// connected.
fn observe_pre_generate_hook(iii: &IIIClient) -> ObserverHandle {
    iii.observe(
        Source::function(PRE_GENERATE_HOOK_FN).args(pre_generate_hook_args()),
        crate::functions::inject_guidance::GUIDANCE_HOOK_ID,
    )
}

/// Observe the new sweep expression, then dispose the old observer
/// (observe-new-then-dispose-old: a fail-safe overlap). Returns `false`, with
/// the old observer kept, when the cron worker refuses the new expression.
/// A replacement still `pending` after [`SWEEP_READY_WAIT`] (the cron worker
/// is down) is kept: it becomes active when the cron worker registers.
async fn replace_sweep(iii: &IIIClient, handles: &ObserverHandles, cfg: &WorkerConfig) -> bool {
    let new = observe_sweep(iii, cfg);
    let ready = tokio::time::timeout(SWEEP_READY_WAIT, new.ready()).await;
    if let Ok(Err(e)) = ready {
        tracing::error!(error = %e, "workflow sweep observer refused");
        dispose(new).await;
        return false;
    }
    let old = std::mem::replace(
        &mut *handles.sweep.lock().unwrap_or_else(|p| p.into_inner()),
        new,
    );
    dispose(old).await;
    true
}

async fn dispose(handle: ObserverHandle) {
    if let Err(e) = handle.dispose().await {
        tracing::warn!(error = %e, observer_id = handle.id(), "observer dispose failed");
    }
}

/// Register the internal `workflow::on-config-change` handler and observe
/// `configuration::on-updated` with it. `handles` holds the live sweep
/// observer the handler replaces when `sweep_expression` changes. Every
/// delivery runs the reload under the shared lock (fetch inside it); the
/// returned [`config_client::Reload`] lets boot run one extra pass to close
/// the fetch→observe gap.
pub fn observe_config_changes(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    handles: Arc<ObserverHandles>,
) -> config_client::Reload {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        config_id(),
        CONFIG_FN_ID,
        "Internal: hot-reload workflow config — replaces the cron sweep observer on a \
         sweep_expression change and swaps the per-call tuning snapshot otherwise.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            let handles = handles.clone();
            async move { on_config_change(&engine, &cell, &handles).await }
        },
    )
}

/// Reload from the AUTHORITATIVE configuration. The caller-supplied event
/// payload is intentionally ignored: a direct call can never inject config.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, handles: &ObserverHandles) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "config-change: fetch failed; keeping previous config");
            return;
        }
    };

    let old = cell.read().await.clone();
    // Replace the sweep observer only when a boot-relevant field changes — one
    // baked into a live observer's args rather than hot-applied per call. Today
    // that is only the sweep schedule. Commit the new expression into the live
    // snapshot ONLY when the cron worker accepts it; on a refusal keep the old
    // expression so the next config-change still sees a diff and retries —
    // otherwise the snapshot would advertise a schedule the active observer
    // isn't running.
    let mut applied = cfg;
    if old.sweep_expression != applied.sweep_expression {
        if replace_sweep(iii, handles, &applied).await {
            tracing::info!("workflow sweep observer replaced (sweep_expression changed)");
        } else {
            applied.sweep_expression = old.sweep_expression.clone();
            tracing::error!(
                "workflow sweep replacement refused; keeping the previous sweep observer and schedule"
            );
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

    #[test]
    fn observers_carry_the_v2_args() {
        assert_eq!(
            sweep_args(&WorkerConfig {
                sweep_expression: "0 */5 * * * *".into(),
                ..WorkerConfig::default()
            }),
            json!({ "expression": "0 */5 * * * *" })
        );
        assert_eq!(
            pre_call_hook_args(),
            json!({ "functions": ["workflow::start"], "on_error": "fail_open", "timeout_ms": 30000 })
        );
        let guidance = pre_generate_hook_args();
        assert_eq!(guidance["on_error"], "fail_open");
        assert_eq!(
            guidance["inject_prompt"],
            crate::functions::inject_guidance::WORKFLOW_GUIDANCE
        );
    }

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
