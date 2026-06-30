use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::WorkerConfig;
use crate::locks::WorkflowLocks;

pub mod inject_guidance;
pub mod node_result;
pub mod stamp_reply;
pub mod start;
pub mod status;
pub mod stop;
pub mod sweep;
pub mod tick;
pub mod wake;

pub type ConfigCell = Arc<tokio::sync::RwLock<Arc<WorkerConfig>>>;

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub cfg: ConfigCell,
    pub locks: WorkflowLocks,
}

impl Deps {
    pub async fn cfg(&self) -> Arc<WorkerConfig> {
        self.cfg.read().await.clone()
    }

    pub fn now_ms(&self) -> i64 {
        crate::ids::now_ms()
    }
}

/// Best-effort `harness::stop` of a node's live session. A lost stop is recovered
/// by the run's finalizing/sweep path, so the error is intentionally dropped.
/// `harness::stop` is a no-op for a session with no live turn, so this is safe to
/// call on an already-finished session.
pub(crate) async fn harness_stop_session(deps: &Deps, session_id: &str, dispatch_timeout_ms: u64) {
    let _ = deps
        .iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "harness::stop".into(),
            payload: serde_json::json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(dispatch_timeout_ms),
        })
        .await;
}

/// Stop the live session of every node still `Running`. Workflow nodes are
/// independent top-level turns (NOT ParentLink children), so harness::stop's own
/// live_children cascade never reaches them — we enumerate the recorded node
/// sessions ourselves. Used by both `workflow::stop` (explicit cancel) and
/// `finalize` (a run reaching a terminal state while sibling nodes are still in
/// flight): without it, a failed/cancelled/completed run leaves orphaned agent
/// turns running, burning tokens on output nobody will read. harness::stop is a
/// no-op for a session with no live turn, so a double-stop is safe.
pub(crate) async fn cascade_stop_running(
    deps: &Deps,
    record: &crate::types::WorkflowRunRecord,
    dispatch_timeout_ms: u64,
) {
    for cp in record.nodes.values() {
        if matches!(cp.state, crate::types::NodeState::Running) {
            if let Some(sid) = &cp.session_id {
                harness_stop_session(deps, sid, dispatch_timeout_ms).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TickRequest / TickResponse (shared between mod.rs registration and tick.rs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TickRequest {
    pub run_id: String,
    #[serde(default)]
    pub step: u64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TickResponse {
    pub skipped: bool,
}

// ---------------------------------------------------------------------------
// register_all
// ---------------------------------------------------------------------------

pub fn register_all(iii: &Arc<IIIClient>, deps: &Deps) {
    let d = deps.clone();
    iii.register_function(
        "workflow::start",
        RegisterFunction::new_async(move |req: start::StartRequest| {
            let d = d.clone();
            async move { start::handle(&d, req).await.map_err(Error::from) }
        })
        .description(
            "Launch a declarative multi-agent DAG (fan-out + barrier/join, durable & \
             crash-resumable) and return its run_id immediately. Use this to ORCHESTRATE \
             MULTIPLE distinct agents whose pipeline you can draw up front. Do NOT use it for \
             work you can finish in the current turn: if you already hold the inputs you need \
             (e.g. you were handed results to synthesize), produce the answer DIRECTLY — never \
             wrap a single reasoning step in a one-node workflow. For a genuine pipeline it runs \
             for as long as it needs without blocking the caller. PREFER `notify` (a function_id \
             pushed {run_id, status, result, result_error} once the run is terminal) over polling: \
             each workflow::status call costs one of your turns and a poll loop can exhaust your \
             turn budget. Use workflow::status only for the occasional check, not a tight loop. \
             Request shape: \
             `{\"definition\":{\"version\":1,\"nodes\":{\"<id>\":{\"agent\":{\"model\":\"<id from \
             router::models::list>\"},\"input\":{\"from\":\"run_input\"}}},\"output\":{\"from\":\"node:<id>\"}},\
             \"notify\":{\"function_id\":\"...\"},\"reply_to\":{}}` — `nodes` is an OBJECT \
             keyed by node id (NOT an array); each node is `{agent, input, depends_on?, fanout?}`. To get \
             the result: set `reply_to:{}` (or notify) \
             and then END YOUR TURN — do NOT claim a result was delivered or produce one this turn, it \
             arrives as a separate message when the run finishes. Never poll workflow::status in a loop. \
             Full field docs are inline in this function's request schema.",
        ),
    );

    let d = deps.clone();
    iii.register_function(
        "workflow::tick",
        RegisterFunction::new_async(move |req: TickRequest| {
            let d = d.clone();
            async move { tick::handle(&d, req).await.map_err(Error::from) }
        })
        .description("Internal durable workflow step."),
    );
    let d = deps.clone();
    iii.register_function(
        "workflow::status",
        RegisterFunction::new_async(move |req: status::StatusRequest| {
            let d = d.clone();
            async move { status::handle(&d, req).await.map_err(Error::from) }
        })
        .description(
            "Return the current status of a workflow run (state per node, plus node_errors / \
             node_results / result / result_error), or null if not found. A single check is \
             cheap, but each call costs one of your turns — do NOT poll in a loop waiting for a \
             long run to finish, that exhausts your turn budget. To be pushed the outcome when \
             the run reaches a terminal state, pass `notify` to workflow::start instead.",
        ),
    );

    let d = deps.clone();
    iii.register_function(
        "workflow::node-result",
        RegisterFunction::new_async(move |req: node_result::NodeResultRequest| {
            let d = d.clone();
            async move { node_result::handle(&d, req).await.map_err(Error::from) }
        })
        .description(
            "Fetch the stored JSON result of a single node by its uid — the `node_uid` arg \
             (alias `uid`): a node_id, or '{node_id}#{i}' for a fanned-out item. Use it to \
             recover partial outputs after a run fails partway: workflow::status lists the uids \
             that have a result under `node_results`. Returns {result: null} if the node has not \
             completed or has no stored result.",
        ),
    );
    let d = deps.clone();
    iii.register_function(
        "workflow::stop",
        RegisterFunction::new_async(move |req: stop::StopRequest| {
            let d = d.clone();
            async move { stop::handle(&d, req).await.map_err(Error::from) }
        })
        .description("Cooperatively cancel a workflow run and cascade harness::stop to each live node session."),
    );
    let d = deps.clone();
    iii.register_function(
        sweep::SWEEP_ID,
        RegisterFunction::new_async(move |req: sweep::SweepEvent| {
            let d = d.clone();
            async move { sweep::handle(&d, req).await.map_err(Error::from) }
        })
        .description(sweep::SWEEP_DESC),
    );
    let d = deps.clone();
    iii.register_function(
        wake::WAKE_ID,
        RegisterFunction::new_async(move |req: wake::WakeEvent| {
            let d = d.clone();
            async move { wake::handle(&d, req).await.map_err(Error::from) }
        })
        .description(wake::WAKE_DESC),
    );

    iii.register_function(
        stamp_reply::STAMP_REPLY_ID,
        RegisterFunction::new_async(move |event: stamp_reply::StampReplyEvent| async move {
            stamp_reply::handle(event).await
        })
        .description(
            "Internal pre_trigger hook on workflow::start: stamps the caller's session (and, \
             when the call sets `reply_to`, the caller's model/provider/policy) into the \
             arguments so the run nests under the orchestrator and can deliver its outcome. \
             Always continues — never blocks the call. Bound to harness::hook::pre-trigger; \
             not called directly.",
        ),
    );

    iii.register_function(
        inject_guidance::GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(move |event: inject_guidance::PreGenerateEvent| async move {
            inject_guidance::handle(event).await
        })
        .description(inject_guidance::GUIDANCE_HOOK_DESC),
    );
}
