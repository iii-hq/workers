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
pub mod submit;
pub mod sweep;
pub mod tick;
pub mod validate_result;
pub mod wake;

pub type ConfigCell = Arc<tokio::sync::RwLock<Arc<WorkerConfig>>>;

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub cfg: ConfigCell,
    pub locks: WorkflowLocks,
    /// Where `workflow::on-run-completed` goes: the client in production.
    pub events: Arc<dyn crate::events::EventDeliverer>,
}

impl Deps {
    pub async fn cfg(&self) -> Arc<WorkerConfig> {
        self.cfg.read().await.clone()
    }

    pub fn now_ms(&self) -> i64 {
        crate::ids::now_ms()
    }
}

/// Best-effort `harness::stop` of a node's session tree, then disposal of the tree's
/// observers: a stopped session parked on a wake still wakes on a delivery, while
/// disposing its last wake after the stop ends it as cancelled with no new turn.
/// Errors are only logged and never retried here. Only live checkpoints get a second
/// pass, from `cascade_stop_running` in the finalizing tick after `workflow::stop`. A
/// replaced (refire) or failed-out session whose stop or disposal fails keeps its
/// observers, so it can still wake unless they expire.
pub(crate) async fn harness_stop_session(deps: &Deps, session_id: &str, dispatch_timeout_ms: u64) {
    let iii = deps.iii.clone();
    let call = move |function_id: &'static str, payload: serde_json::Value| {
        let iii = iii.clone();
        async move {
            iii.call(iii_sdk::CallRequest {
                function_id: function_id.into(),
                payload,
                action: None,
                timeout_ms: Some(dispatch_timeout_ms),
            })
            .await
        }
    };
    let Some(sessions) = stop_tree(session_id, &call).await else {
        return;
    };
    let root = session_id.to_string();
    // Best-effort and off the run lock: the disposal is lost if the worker exits mid-task.
    tokio::spawn(async move { dispose_tree(&root, sessions, &call).await });
}

/// Read the session tree, then `harness::stop` it. The sessions to dispose, or `None`
/// when the stop failed.
async fn stop_tree<F, Fut>(session_id: &str, call: &F) -> Option<Vec<String>>
where
    F: Fn(&'static str, serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, Error>>,
{
    // Tree first: a session a finishing spawn adds later is not flagged by the stop, so disposing its wake starts a turn.
    let tree = serde_json::json!({ "root_session_id": session_id });
    let sessions = match call("harness::session-tree", tree).await {
        Ok(resp) => listed_ids(&resp, "sessions", "session_id"),
        Err(error) => {
            tracing::warn!(session_id, %error, "harness::session-tree failed; root only");
            vec![session_id.to_string()]
        }
    };
    let stop = serde_json::json!({ "session_id": session_id, "quiet_deadlines": true });
    if let Err(error) = call("harness::stop", stop).await {
        // A wake disposed from a session the stop did not flag wakes it with a new turn.
        tracing::warn!(session_id, %error, "harness::stop failed; keeping its observers");
        return None;
    }
    Some(sessions)
}

/// Dispose every observer of `sessions` (read before the stop), then stop the root again.
async fn dispose_tree<F, Fut>(session_id: &str, sessions: Vec<String>, call: &F)
where
    F: Fn(&'static str, serde_json::Value) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, Error>>,
{
    for sid in sessions {
        let list = serde_json::json!({ "session_id": sid });
        let observers = match call("harness::observers::list", list).await {
            Ok(resp) => listed_ids(&resp, "observers", "observer_id"),
            Err(error) => {
                tracing::warn!(session_id = %sid, %error, "harness::observers::list failed");
                continue;
            }
        };
        for observer_id in observers {
            let dispose = serde_json::json!({ "session_id": sid, "observer_id": observer_id });
            if let Err(error) = call("harness::observers::dispose", dispose).await {
                tracing::warn!(session_id = %sid, observer_id, %error, "observer dispose failed");
            }
        }
    }
    // A delivery between the stop and the disposal starts a fresh, unflagged turn; stop it too.
    let stop = serde_json::json!({ "session_id": session_id, "quiet_deadlines": true });
    if let Err(error) = call("harness::stop", stop).await {
        tracing::warn!(session_id, %error, "second harness::stop failed");
    }
}

/// The `field` of each row of `resp[list]` (`harness::session-tree` sessions,
/// `harness::observers::list` observers). Pure.
fn listed_ids(resp: &serde_json::Value, list: &str, field: &str) -> Vec<String> {
    resp[list]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row[field].as_str().map(str::to_string))
        .collect()
}

/// Stop the session of every live node (`NodeCheckpoint::is_live`). Node
/// sessions sit in the caller's session tree (`metadata.parent_session_id`), so
/// stopping the caller stops every node; but stopping a run does not stop its
/// caller, and a run without a caller has no tree, so the run stops its recorded
/// node sessions itself. Used by both `workflow::stop` (explicit cancel) and
/// `finalize` (a run reaching a terminal state while sibling nodes are still in
/// flight): without it, a failed/cancelled/completed run leaves orphaned agent
/// turns running, burning tokens on output nobody will read. harness::stop is a
/// no-op for a session with no live turn, so a double-stop is safe.
pub(crate) async fn cascade_stop_running(
    deps: &Deps,
    record: &crate::types::WorkflowRunRecord,
    dispatch_timeout_ms: u64,
) {
    for cp in record.nodes.values().filter(|cp| cp.is_live()) {
        if let Some(sid) = &cp.session_id {
            harness_stop_session(deps, sid, dispatch_timeout_ms).await;
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
    /// Sent by `workflow::wake`: runs even below the step floor.
    #[serde(default)]
    pub wake: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TickResponse {
    pub skipped: bool,
}

// ---------------------------------------------------------------------------
// register_all
// ---------------------------------------------------------------------------

/// Register `workflow::start`. Boot calls it after the `workflow::stamp-reply`
/// observer is active (or a bounded wait ran out), so an agent's call cannot
/// reach the worker unstamped while the worker connects.
pub fn register_start(iii: &Arc<IIIClient>, deps: &Deps) {
    let d = deps.clone();
    iii.register_function(
        "workflow::start",
        RegisterFunction::new_async(move |req: start::StartRequest| {
            let d = d.clone();
            async move { start::handle(&d, req).await.map_err(Error::from) }
        })
        .description(
            "Run a multi-agent workflow and return {run_id, next} immediately. Declarative DAG: \
             fan-out, barrier/join, durable, crash-resumable. Shape: \
             {\"nodes\":{\"a\":{\"task\":\"…\"},\"b\":{\"task\":\"…\",\"reads\":\"a\"}}}; a node \
             omits `model` to run on your model. To get the result make the `harness::observe` \
             call that `next` gives and END YOUR TURN — it arrives as a `[notification]` when the \
             run ends; workers pass `notify`. Never poll workflow::status in a loop.",
        ),
    );
}

/// Register every function but `workflow::start` (see [`register_start`]).
pub fn register_all(iii: &Arc<IIIClient>, deps: &Deps) {
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
            "Return a snapshot of a workflow run (status, per-node state with unstarted nodes as \
             pending, node_errors, node_results, phases, result, result_error), or null if not \
             found. `details: true` adds node_details: label, phase, model, session, timings, live \
             usage and harness status per node (at most 100 nodes). Each call costs a turn: do not \
             poll in a loop; observe `workflow::on-run-completed` with args `{run_id}`, or pass \
             `notify` to workflow::start, to be pushed the outcome.",
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
            "Fetch the stored JSON result of one node: `node_uid` is the node id, \
             '{node_id}#{i}' for a fanout item, or '{node_id}#it{k}' for loop pass k ('#it0' is \
             the first pass; the bare id of a node in a loop is its last pass; workflow::status \
             lists the done passes under `node_results`). Returns {result: null} when that node \
             is not done.",
        ),
    );
    let d = deps.clone();
    iii.register_function(
        "workflow::stop",
        RegisterFunction::new_async(move |req: stop::StopRequest| {
            let d = d.clone();
            async move { stop::handle(&d, req).await.map_err(Error::from) }
        })
        .description("Cancel a workflow run, cascading harness::stop to each live node session."),
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
        RegisterFunction::new_async(
            move |delivered: crate::observed::Delivered<wake::WakeEvent>| {
                let d = d.clone();
                let req = crate::observed::delivered_event(delivered).unwrap_or_default();
                async move { wake::handle(&d, req).await.map_err(Error::from) }
            },
        )
        .description(wake::WAKE_DESC),
    );

    iii.register_function(
        stamp_reply::STAMP_REPLY_ID,
        RegisterFunction::new_async(move |event: stamp_reply::StampReplyEvent| async move {
            stamp_reply::handle(event).await
        })
        .description(
            "Internal pre_call hook on workflow::start: stamps the caller's session and folder \
             into the arguments. Always continues; not called directly.",
        ),
    );

    let d = deps.clone();
    iii.register_function(
        submit::SUBMIT_ID,
        RegisterFunction::new_async(move |req: submit::SubmitRequest| {
            let d = d.clone();
            async move { submit::handle(&d, req).await }
        })
        .description(submit::SUBMIT_DESC),
    );

    let d = deps.clone();
    iii.register_function(
        validate_result::VALIDATE_RESULT_ID,
        RegisterFunction::new_async(move |event: validate_result::PostTurnEvent| {
            let d = d.clone();
            async move { validate_result::handle(&d, event).await }
        })
        .description(validate_result::VALIDATE_RESULT_DESC),
    );

    crate::events::register_run_completed(iii, deps);

    iii.register_function(
        inject_guidance::GUIDANCE_HOOK_ID,
        RegisterFunction::new_async(move |event: inject_guidance::PreGenerateEvent| async move {
            inject_guidance::handle(event).await
        })
        .description(inject_guidance::GUIDANCE_HOOK_DESC),
    );
}

#[cfg(test)]
mod tests {
    use super::listed_ids;
    use serde_json::{json, Value};

    #[test]
    fn listed_ids_reads_tree_sessions_and_observer_ids() {
        let tree = json!({
            "root_session_id": "wf_r_n@r0",
            "sessions": [
                {"session_id": "wf_r_n@r0", "depth": 0},
                {"session_id": "child", "parent_session_id": "wf_r_n@r0", "depth": 1}
            ],
            "complete": true
        });
        assert_eq!(
            listed_ids(&tree, "sessions", "session_id"),
            ["wf_r_n@r0", "child"]
        );
        let observers = json!({"observers": [
            {"observer_id": "b1", "function_id": "state::on-changed", "once": true, "fires": 0},
            {"function_id": "no-id"},
            {"observer_id": "b2", "target": "x::y", "once": false, "fires": 3}
        ]});
        assert_eq!(
            listed_ids(&observers, "observers", "observer_id"),
            ["b1", "b2"]
        );
        // An unknown session (`null`) or an empty tree lists nothing.
        assert!(listed_ids(&Value::Null, "observers", "observer_id").is_empty());
        assert!(listed_ids(&json!({"sessions": []}), "sessions", "session_id").is_empty());
    }

    /// The calls `harness_stop_session` makes against a scripted harness: whether
    /// `harness::stop` fails, the tree (`None`: session-tree fails), one observer per session.
    async fn stop_calls(stop_fails: bool, tree: Option<Value>) -> Vec<(&'static str, Value)> {
        let calls = std::cell::RefCell::new(Vec::new());
        let call = |function_id: &'static str, payload: Value| {
            calls.borrow_mut().push((function_id, payload.clone()));
            std::future::ready(match function_id {
                "harness::stop" if stop_fails => Err(iii_sdk::errors::Error::Timeout),
                "harness::session-tree" => tree.clone().ok_or(iii_sdk::errors::Error::Timeout),
                "harness::observers::list" => Ok(
                    json!({"observers": [{"observer_id": format!("w-{}", payload["session_id"].as_str().unwrap())}]}),
                ),
                _ => Ok(Value::Null),
            })
        };
        if let Some(sessions) = super::stop_tree("root", &call).await {
            super::dispose_tree("root", sessions, &call).await;
        }
        calls.into_inner()
    }

    fn ids(calls: &[(&'static str, Value)]) -> Vec<&'static str> {
        calls.iter().map(|(id, _)| *id).collect()
    }

    #[tokio::test]
    async fn stop_session_disposes_only_after_a_successful_stop() {
        let tree = json!({"sessions": [{"session_id": "root"}, {"session_id": "child"}]});

        // A failed stop disposes nothing: a wake disposed from an unflagged session wakes it.
        let calls = stop_calls(true, Some(tree.clone())).await;
        assert_eq!(ids(&calls), ["harness::session-tree", "harness::stop"]);

        // No tree: the root alone, then the second stop.
        let calls = stop_calls(false, None).await;
        assert_eq!(
            ids(&calls),
            [
                "harness::session-tree",
                "harness::stop",
                "harness::observers::list",
                "harness::observers::dispose",
                "harness::stop"
            ]
        );
        assert_eq!(
            calls[3].1,
            json!({"session_id": "root", "observer_id": "w-root"})
        );

        // The tree read before the stop is disposed after it, every session, then stopped again.
        let calls = stop_calls(false, Some(tree)).await;
        let first_stop = ids(&calls)
            .iter()
            .position(|id| *id == "harness::stop")
            .unwrap();
        let disposed: Vec<&Value> = calls
            .iter()
            .enumerate()
            .filter(|(_, (id, _))| *id == "harness::observers::dispose")
            .map(|(i, (_, payload))| {
                assert!(i > first_stop, "dispose before the stop");
                payload
            })
            .collect();
        assert_eq!(
            disposed,
            [
                &json!({"session_id": "root", "observer_id": "w-root"}),
                &json!({"session_id": "child", "observer_id": "w-child"})
            ]
        );
        assert_eq!(ids(&calls)[0], "harness::session-tree");
        assert_eq!(
            calls.last().unwrap(),
            &(
                "harness::stop",
                json!({"session_id": "root", "quiet_deadlines": true})
            )
        );
    }
}
