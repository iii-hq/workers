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
            "Launch a declarative multi-agent DAG (fan-out, barrier/join, durable and \
             crash-resumable) and return its run_id immediately. To get the result set \
             `reply_to: {}` (or `notify`) and END YOUR TURN — it arrives as a separate message \
             when the run finishes; never poll workflow::status in a loop.",
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
            "Return a snapshot of a workflow run (status, per-node state, node_errors, \
             node_results, result, result_error), or null if not found. Each call costs a turn: \
             do not poll in a loop; pass `reply_to`/`notify` to workflow::start to be pushed the \
             outcome.",
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
            "Fetch the stored JSON result of one node: `node_uid` is the node id, or \
             '{node_id}#{i}' for a fanout item (workflow::status lists them under \
             `node_results`). Returns {result: null} when nothing is stored.",
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
             for `reply_to`, model/provider/policy) into the arguments. Always continues; not \
             called directly.",
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
