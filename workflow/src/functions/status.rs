use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    error::WorkflowError,
    state,
    types::{NodeDef, NodeState, RunStatus, WorkflowDef, WorkflowRunRecord},
};

use super::Deps;

/// Timeout of each live `harness::metrics` / `harness::status` read.
const LIVE_TIMEOUT_MS: u64 = 5_000;
/// Live node reads in flight at once.
const LIVE_CONCURRENCY: usize = 16;
/// Most nodes `node_details` describes.
const MAX_DETAIL_NODES: usize = 100;

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

/// Read a single snapshot of a workflow run's status.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StatusRequest {
    /// The `run_id` returned by `workflow::start`.
    pub run_id: String,
    /// Also return `node_details` (at most 100 nodes: running first, then pending,
    /// failed, cancelled, done). Reads each node's usage and harness status live.
    #[serde(default)]
    pub details: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StatusResponse {
    pub run_id: String,
    pub status: RunStatus,
    /// Per-node state keyed by node uid. Definition nodes that have not started
    /// are `pending`; an unexpanded fanout is listed once under its node id.
    pub nodes: BTreeMap<String, NodeState>,
    /// Errors for nodes that failed, keyed by node uid — e.g. "no provider
    /// registered for model …". Present so a caller can diagnose a `failed` run
    /// without digging into stored state. Omitted when no node carries an error.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_errors: BTreeMap<String, String>,
    /// Nodes that have a stored result: node uid → result_ref (its key in state).
    /// Fetch the value with `workflow::node-result { run_id, node_uid }`. Lets a
    /// caller recover partial work from a run that failed partway (the run-level
    /// `result` is only set on a Completed run). Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_results: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Run-level failure summary (set when `status == failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Node uid counts per definition `phase` (`failed` includes cancelled); nodes
    /// without a phase are left out. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub phases: BTreeMap<String, PhaseCounts>,
    /// Only with `details: true`: per-node detail keyed by node uid.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_details: BTreeMap<String, NodeDetail>,
    /// `node_details` was cut at 100 nodes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub details_truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PhaseCounts {
    pub total: u32,
    pub done: u32,
    pub failed: u32,
    pub running: u32,
}

/// One node in `node_details`. Definition fields come from the run's definition;
/// `harness_status`, `expects_wake`, `last_generate_at`, `usage` and an observed
/// `model` are read live and omitted when that read fails.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NodeDetail {
    pub state: NodeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// The model the node session last ran; for a node without a profile, the
    /// definition's model until one is observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The current attempt's session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 0 for the first attempt; each timeout refire adds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Epoch ms of the current attempt's send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    /// Epoch ms the node ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// `harness::status` turn status; running nodes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_status: Option<String>,
    /// The session waits on an armed wake; running nodes only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expects_wake: Option<bool>,
    /// Epoch ms of the last model request anywhere in the node's session tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_generate_at: Option<i64>,
    /// `harness::metrics` totals of the current attempt's session tree, plus
    /// `complete`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

/// What the live harness reads gave for one node session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveNode {
    pub model: Option<String>,
    pub last_generate_at: Option<i64>,
    pub usage: Option<Value>,
    pub harness_status: Option<String>,
    pub expects_wake: Option<bool>,
}

// ---------------------------------------------------------------------------
// Pure assembly
// ---------------------------------------------------------------------------

/// The definition node of `uid` (a fanout item `<id>#<i>` or a loop pass
/// `<id>#it<k>` maps to `<id>`).
fn node_def<'a>(def: Option<&'a WorkflowDef>, uid: &str) -> Option<&'a NodeDef> {
    def?.nodes.get(uid.split('#').next()?)
}

/// Every checkpoint's state, plus each definition node with no checkpoint as
/// Pending. An expanded fanout is listed only through its items (none for an
/// empty array) or its failed-expansion checkpoint.
fn node_states(
    record: &WorkflowRunRecord,
    def: Option<&WorkflowDef>,
) -> BTreeMap<String, NodeState> {
    let mut nodes: BTreeMap<String, NodeState> = record
        .nodes
        .iter()
        .map(|(uid, cp)| (uid.clone(), cp.state))
        .collect();
    for (id, node) in def.iter().flat_map(|d| &d.nodes) {
        if node.fanout.is_none() || !record.fanout_src.contains_key(id) {
            nodes.entry(id.clone()).or_insert(NodeState::Pending);
        }
    }
    nodes
}

fn phase_counts(
    nodes: &BTreeMap<String, NodeState>,
    def: Option<&WorkflowDef>,
) -> BTreeMap<String, PhaseCounts> {
    let mut phases: BTreeMap<String, PhaseCounts> = BTreeMap::new();
    for (uid, state) in nodes {
        let Some(phase) = node_def(def, uid).and_then(|n| n.phase.as_ref()) else {
            continue;
        };
        let c = phases.entry(phase.clone()).or_default();
        c.total += 1;
        match state {
            NodeState::Done => c.done += 1,
            NodeState::Failed | NodeState::Cancelled => c.failed += 1,
            NodeState::Running => c.running += 1,
            NodeState::Pending => {}
        }
    }
    phases
}

/// The uids `node_details` covers, in order, and whether the list was cut.
fn detail_uids(nodes: &BTreeMap<String, NodeState>) -> (Vec<&String>, bool) {
    let rank = |s: NodeState| match s {
        NodeState::Running => 0,
        NodeState::Pending => 1,
        NodeState::Failed => 2,
        NodeState::Cancelled => 3,
        NodeState::Done => 4,
    };
    let mut uids: Vec<&String> = nodes.keys().collect();
    // Stable: uid order within a state.
    uids.sort_by_key(|uid| rank(nodes[*uid]));
    let truncated = uids.len() > MAX_DETAIL_NODES;
    uids.truncate(MAX_DETAIL_NODES);
    (uids, truncated)
}

fn node_detail(
    uid: &str,
    state: NodeState,
    record: &WorkflowRunRecord,
    def: Option<&WorkflowDef>,
    live: Option<&LiveNode>,
) -> NodeDetail {
    let node = node_def(def, uid);
    let cp = record.nodes.get(uid);
    let session_id = cp.and_then(|cp| cp.session_id.clone());
    let live = live.cloned().unwrap_or_default();
    // A profile may pin another model, so its definition model is not shown as fact.
    let def_model = node
        .filter(|n| n.agent.profile.is_none() && !n.agent.model.is_empty())
        .map(|n| n.agent.model.clone());
    NodeDetail {
        state,
        label: node.and_then(|n| n.label.clone()),
        phase: node.and_then(|n| n.phase.clone()),
        model: live.model.or(def_model),
        profile: node.and_then(|n| n.agent.profile.clone()),
        attempt: session_id.as_ref().and(cp.map(|cp| cp.retries)),
        session_id,
        started_at: cp.and_then(|cp| cp.pending_at),
        finished_at: cp.and_then(|cp| cp.finished_at),
        harness_status: live.harness_status,
        expects_wake: live.expects_wake,
        last_generate_at: live.last_generate_at,
        usage: live.usage,
    }
}

/// The status snapshot of `record`. Without `def` (missing definition) it lists
/// the checkpoints only, with no phases or definition fields. Pure.
pub(crate) fn build_status(
    record: WorkflowRunRecord,
    def: Option<&WorkflowDef>,
    live: &BTreeMap<String, LiveNode>,
    details: bool,
) -> StatusResponse {
    let nodes = node_states(&record, def);
    let phases = phase_counts(&nodes, def);
    let (node_details, details_truncated) = if details {
        let (uids, truncated) = detail_uids(&nodes);
        let detail = |uid: &String| node_detail(uid, nodes[uid], &record, def, live.get(uid));
        (
            uids.into_iter()
                .map(|uid| (uid.clone(), detail(uid)))
                .collect(),
            truncated,
        )
    } else {
        (BTreeMap::new(), false)
    };
    let node_errors = record
        .nodes
        .iter()
        .filter_map(|(uid, cp)| cp.result_error.clone().map(|e| (uid.clone(), e)))
        .collect();
    let node_results = record
        .nodes
        .iter()
        .filter_map(|(uid, cp)| cp.result_ref.clone().map(|r| (uid.clone(), r)))
        .collect();

    StatusResponse {
        run_id: record.run_id,
        status: record.status,
        nodes,
        node_errors,
        node_results,
        result: record.result,
        result_error: record.result_error,
        created_at: record.created_at,
        updated_at: record.updated_at,
        phases,
        node_details,
        details_truncated,
    }
}

/// `harness::metrics {session_id, subtree, omit_traces}` → usage (totals without
/// `by_model`, plus `complete`), the node session's `context.model`, and the
/// latest `context.timestamp` over the whole tree. Pure.
fn parse_metrics(resp: &Value, sid: &str) -> LiveNode {
    let rows = resp["by_session"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let usage = resp["totals"].as_object().map(|totals| {
        let mut usage = totals.clone();
        usage.remove("by_model");
        usage.insert("complete".into(), resp["complete"].clone());
        Value::Object(usage)
    });
    LiveNode {
        model: rows
            .iter()
            .find(|row| row["session_id"] == sid)
            .and_then(|row| row["context"]["model"].as_str())
            .map(str::to_string),
        last_generate_at: rows
            .iter()
            .filter_map(|row| row["context"]["timestamp"].as_i64())
            .max(),
        usage,
        ..Default::default()
    }
}

/// `harness::status` → (turn status, expects_wake); both `None` for an unknown
/// session (`null`). Pure.
fn parse_harness_status(resp: &Value) -> (Option<String>, Option<bool>) {
    (
        resp["status"].as_str().map(str::to_string),
        resp["expects_wake"].as_bool(),
    )
}

// ---------------------------------------------------------------------------
// Live reads
// ---------------------------------------------------------------------------

/// A node session to read live: (uid, session_id, running).
type LiveTarget = (String, String, bool);

fn live_targets(record: &WorkflowRunRecord, def: Option<&WorkflowDef>) -> Vec<LiveTarget> {
    let nodes = node_states(record, def);
    detail_uids(&nodes)
        .0
        .into_iter()
        .filter_map(|uid| {
            let cp = record.nodes.get(uid)?;
            // Only a send that reached the harness stamps `pending_at`; a refused one made no session.
            Some((
                uid.clone(),
                cp.pending_at.and(cp.session_id.clone())?,
                cp.state == NodeState::Running,
            ))
        })
        .collect()
}

async fn fetch_one(iii: &iii_sdk::IIIClient, sid: &str, running: bool) -> LiveNode {
    let call = |function_id: &str, payload: Value| {
        iii.call(iii_sdk::CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(LIVE_TIMEOUT_MS),
        })
    };
    let metrics = call(
        "harness::metrics",
        json!({ "session_id": sid, "subtree": true, "omit_traces": true }),
    );
    let status = async {
        if running {
            call("harness::status", json!({ "session_id": sid }))
                .await
                .ok()
        } else {
            None
        }
    };
    let (metrics, status) = tokio::join!(metrics, status);
    let mut live = metrics
        .map(|resp| parse_metrics(&resp, sid))
        .unwrap_or_default();
    if let Some(resp) = status {
        (live.harness_status, live.expects_wake) = parse_harness_status(&resp);
    }
    live
}

/// Live reads for `targets`, [`LIVE_CONCURRENCY`] at a time. A failed read leaves
/// its fields out; nothing is persisted.
async fn fetch_live(deps: &Deps, targets: Vec<LiveTarget>) -> BTreeMap<String, LiveNode> {
    let mut live = BTreeMap::new();
    for chunk in targets.chunks(LIVE_CONCURRENCY) {
        let mut set = tokio::task::JoinSet::new();
        for (uid, sid, running) in chunk.iter().cloned() {
            let iii = deps.iii.clone();
            set.spawn(async move { (uid, fetch_one(&iii, &sid, running).await) });
        }
        while let Some(joined) = set.join_next().await {
            if let Ok((uid, node)) = joined {
                live.insert(uid, node);
            }
        }
    }
    live
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

pub async fn handle(
    deps: &Deps,
    req: StatusRequest,
) -> Result<Option<StatusResponse>, WorkflowError> {
    let Some(record) = state::get_run(&deps.iii, &req.run_id).await? else {
        return Ok(None);
    };
    // A missing definition (the GC window: `delete_run` drops it before the run) falls
    // back to the checkpoints; a read error propagates, as for `get_run`.
    let def = state::get_def(&deps.iii, &req.run_id).await?;
    let live = if req.details {
        fetch_live(deps, live_targets(&record, def.as_ref())).await
    } else {
        BTreeMap::new()
    };
    Ok(Some(build_status(record, def.as_ref(), &live, req.details)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeCheckpoint;
    use serde_json::json;

    fn response(nodes: BTreeMap<String, NodeState>) -> StatusResponse {
        StatusResponse {
            run_id: "r_xyz".to_string(),
            status: RunStatus::Running,
            nodes,
            node_errors: BTreeMap::new(),
            node_results: BTreeMap::new(),
            result: None,
            result_error: None,
            created_at: 1,
            updated_at: 2,
            phases: BTreeMap::new(),
            node_details: BTreeMap::new(),
            details_truncated: false,
        }
    }

    #[test]
    fn status_response_serde_round_trip() {
        let mut nodes = BTreeMap::new();
        nodes.insert("plan".to_string(), NodeState::Done);
        nodes.insert("read".to_string(), NodeState::Running);

        let mut resp = response(nodes);
        resp.status = RunStatus::AwaitingNodes;
        resp.node_errors
            .insert("read".to_string(), "boom".to_string());
        resp.node_results.insert(
            "plan".to_string(),
            "workflow_node_result/r_abc123/plan".to_string(),
        );
        resp.result = Some(json!({"summary": "hello"}));
        resp.result_error = Some("node 'read': boom".to_string());
        resp.phases.insert(
            "code".to_string(),
            PhaseCounts {
                total: 2,
                done: 1,
                failed: 0,
                running: 1,
            },
        );
        resp.node_details.insert(
            "read".to_string(),
            NodeDetail {
                state: NodeState::Running,
                label: Some("Read".to_string()),
                usage: Some(json!({"turns": 3, "complete": true})),
                ..Default::default()
            },
        );
        resp.details_truncated = true;

        let serialized = serde_json::to_string(&resp).expect("serialize StatusResponse");
        let decoded: StatusResponse =
            serde_json::from_str(&serialized).expect("deserialize StatusResponse");

        assert_eq!(decoded.run_id, resp.run_id);
        assert_eq!(decoded.status, resp.status);
        assert_eq!(decoded.nodes, resp.nodes);
        assert_eq!(decoded.node_errors, resp.node_errors);
        assert_eq!(decoded.node_results, resp.node_results);
        assert_eq!(decoded.result, resp.result);
        assert_eq!(decoded.result_error, resp.result_error);
        assert_eq!((decoded.created_at, decoded.updated_at), (1, 2));
        assert_eq!(decoded.phases, resp.phases);
        assert_eq!(decoded.node_details, resp.node_details);
        assert!(decoded.details_truncated);
    }

    #[test]
    fn status_response_omits_empty_diagnostics() {
        let serialized = serde_json::to_value(response(BTreeMap::new())).expect("serialize");
        for key in [
            "result",
            "result_error",
            "node_results",
            "node_errors",
            "phases",
            "node_details",
            "details_truncated",
        ] {
            assert!(serialized.get(key).is_none(), "{key} omitted when empty");
        }
        assert_eq!(serialized["created_at"], json!(1));
        assert_eq!(serialized["updated_at"], json!(2));
    }

    /// `plan` → fanout `read` over it → `sum`; `plan` and `read` in phase `code`.
    fn def() -> WorkflowDef {
        serde_json::from_value(json!({
            "nodes": {
                "plan": {"agent": {"model": "m"}, "label": "Plan", "phase": "code",
                         "input": {"from": "run_input"}},
                "read": {"agent": {"profile": "reader", "model": "m"}, "phase": "code",
                         "input": {"from": "fanout_item"}, "depends_on": ["plan"],
                         "fanout": {"over": "node:plan.items"}},
                "sum": {"agent": {"model": "m"}, "input": {"from": "node:read"},
                        "depends_on": ["read"]}
            },
            "output": {"from": "node:sum"}
        }))
        .expect("def")
    }

    fn record(nodes: Value) -> WorkflowRunRecord {
        serde_json::from_value(json!({
            "run_id": "r_1", "step": 1, "status": "awaiting_nodes", "def_ref": "r_1",
            "input": {}, "nodes": nodes, "created_at": 1, "updated_at": 2
        }))
        .expect("record")
    }

    fn states(resp: &StatusResponse) -> Vec<(&str, NodeState)> {
        resp.nodes.iter().map(|(k, v)| (k.as_str(), *v)).collect()
    }

    #[test]
    fn build_status_lists_unstarted_nodes_as_pending() {
        let rec = record(json!({"plan": {"state": "running", "session_id": "s"}}));
        let resp = build_status(rec, Some(&def()), &BTreeMap::new(), false);
        assert_eq!(
            states(&resp),
            vec![
                ("plan", NodeState::Running),
                ("read", NodeState::Pending),
                ("sum", NodeState::Pending)
            ]
        );
        assert_eq!((resp.created_at, resp.updated_at), (1, 2));
    }

    #[test]
    fn build_status_empty_expanded_fanout_is_not_pending() {
        let mut rec = record(json!({"plan": {"state": "done"}}));
        rec.fanout_src.insert("read".into(), vec![]);
        let resp = build_status(rec.clone(), Some(&def()), &BTreeMap::new(), false);
        assert_eq!(
            states(&resp),
            vec![("plan", NodeState::Done), ("sum", NodeState::Pending)]
        );

        // Expanded items replace the base id.
        rec.fanout_src
            .insert("read".into(), vec![json!(1), json!(2)]);
        rec.nodes.insert("read#0".into(), NodeCheckpoint::default());
        rec.nodes.insert("read#1".into(), NodeCheckpoint::default());
        let resp = build_status(rec, Some(&def()), &BTreeMap::new(), false);
        assert!(!resp.nodes.contains_key("read"));
        assert_eq!(resp.nodes["read#1"], NodeState::Pending);
    }

    #[test]
    fn build_status_counts_phases() {
        let mut rec = record(json!({
            "plan": {"state": "done"},
            "read#0": {"state": "running"},
            "read#1": {"state": "cancelled"},
            "read#2": {"state": "pending"}
        }));
        rec.fanout_src
            .insert("read".into(), vec![json!(0), json!(1), json!(2)]);
        let resp = build_status(rec, Some(&def()), &BTreeMap::new(), false);
        assert_eq!(resp.phases.len(), 1, "sum has no phase: {:?}", resp.phases);
        assert_eq!(
            resp.phases["code"],
            PhaseCounts {
                total: 4,
                done: 1,
                failed: 1,
                running: 1
            }
        );
    }

    #[test]
    fn status_lists_each_pass_uid_and_counts_its_phase() {
        let mut rec = record(json!({
            "plan": {"state": "done", "result_ref": "r_1/plan"},
            "plan#it1": {"state": "failed", "result_error": "boom"},
            "plan#it2": {"state": "running", "session_id": "wf_r_1_plan#it2@r0"}
        }));
        rec.fanout_src.insert("read".into(), vec![]);
        let resp = build_status(rec, Some(&def()), &BTreeMap::new(), true);
        assert_eq!(
            states(&resp),
            vec![
                ("plan", NodeState::Done),
                ("plan#it1", NodeState::Failed),
                ("plan#it2", NodeState::Running),
                ("sum", NodeState::Pending)
            ]
        );
        assert_eq!(
            resp.phases["code"],
            PhaseCounts {
                total: 3,
                done: 1,
                failed: 1,
                running: 1
            }
        );
        assert_eq!(resp.node_errors["plan#it1"], "boom");
        assert_eq!(resp.node_results["plan"], "r_1/plan");
        let pass = &resp.node_details["plan#it2"];
        assert_eq!(pass.label.as_deref(), Some("Plan"));
        assert_eq!(pass.phase.as_deref(), Some("code"));
        assert_eq!(pass.model.as_deref(), Some("m"));
        assert_eq!(pass.session_id.as_deref(), Some("wf_r_1_plan#it2@r0"));
    }

    #[test]
    fn build_status_details_only_when_asked() {
        let rec = record(json!({
            "plan": {"state": "done", "session_id": "wf_r_1_plan@r0", "pending_at": 5,
                     "finished_at": 9, "result_ref": "r_1/plan"}
        }));
        let live = BTreeMap::from([(
            "plan".to_string(),
            LiveNode {
                usage: Some(json!({"turns": 2})),
                ..Default::default()
            },
        )]);
        let resp = build_status(rec.clone(), Some(&def()), &live, false);
        assert!(resp.node_details.is_empty());
        assert!(serde_json::to_value(&resp)
            .unwrap()
            .get("node_details")
            .is_none());

        let resp = build_status(rec, Some(&def()), &live, true);
        assert_eq!(
            resp.node_details["plan"],
            NodeDetail {
                state: NodeState::Done,
                label: Some("Plan".into()),
                phase: Some("code".into()),
                model: Some("m".into()),
                session_id: Some("wf_r_1_plan@r0".into()),
                attempt: Some(0),
                started_at: Some(5),
                finished_at: Some(9),
                usage: Some(json!({"turns": 2})),
                ..Default::default()
            }
        );
        // Unstarted: state, label, phase from the definition; nothing live.
        let sum = &resp.node_details["sum"];
        assert_eq!(sum.state, NodeState::Pending);
        assert_eq!((sum.session_id.as_ref(), sum.attempt), (None, None));
        assert!(!resp.details_truncated);
    }

    #[test]
    fn build_status_truncates_details_running_first() {
        let mut nodes = serde_json::Map::new();
        for i in 0..MAX_DETAIL_NODES {
            nodes.insert(format!("d{i:03}"), json!({"state": "done"}));
        }
        nodes.insert("z_run".into(), json!({"state": "running"}));
        nodes.insert("y_fail".into(), json!({"state": "failed"}));
        let resp = build_status(record(Value::Object(nodes)), None, &BTreeMap::new(), true);
        assert!(resp.details_truncated);
        assert_eq!(resp.node_details.len(), MAX_DETAIL_NODES);
        assert!(resp.node_details.contains_key("z_run"));
        assert!(resp.node_details.contains_key("y_fail"));
        // Two non-done nodes took two of the slots; the last done uids were cut.
        assert!(!resp.node_details.contains_key("d099"));
        assert!(!resp.node_details.contains_key("d098"));
        assert!(resp.node_details.contains_key("d097"));
    }

    #[test]
    fn build_status_without_def_falls_back_to_checkpoints() {
        let rec = record(json!({
            "plan": {"state": "failed", "result_error": "boom"},
            "read#0": {"state": "running", "session_id": "s", "retries": 1}
        }));
        let resp = build_status(rec, None, &BTreeMap::new(), true);
        assert_eq!(
            states(&resp),
            vec![("plan", NodeState::Failed), ("read#0", NodeState::Running)]
        );
        assert!(resp.phases.is_empty());
        assert_eq!(resp.node_errors["plan"], "boom");
        let read = &resp.node_details["read#0"];
        assert_eq!(read.attempt, Some(1));
        assert_eq!((read.label.as_ref(), read.model.as_ref()), (None, None));
    }

    #[test]
    fn parse_metrics_reads_totals_model_and_latest_timestamp() {
        let resp = json!({
            "root_session_id": "s",
            "complete": false,
            "totals": {
                "sessions": 2, "turns": 3, "function_calls": 4, "function_call_errors": 0,
                "input_tokens": null, "output_tokens": 7, "cache_read_tokens": null,
                "cache_write_tokens": null, "reasoning_tokens": null, "cost_usd": 0.5,
                "by_model": [{"provider": "p", "model": "m", "turns": 3, "function_calls": 4}]
            },
            "by_session": [
                {"session_id": "child", "depth": 1, "context": {"model": "other", "timestamp": 30}},
                {"session_id": "s", "depth": 0, "context": {"model": "m1", "timestamp": 20}}
            ]
        });
        let live = parse_metrics(&resp, "s");
        assert_eq!(live.model.as_deref(), Some("m1"));
        assert_eq!(live.last_generate_at, Some(30));
        let usage = live.usage.expect("usage");
        assert!(usage.get("by_model").is_none(), "{usage}");
        assert_eq!(usage["complete"], json!(false));
        assert_eq!(usage["input_tokens"], Value::Null);
        assert_eq!(usage["output_tokens"], json!(7));
        assert_eq!(usage["cost_usd"], json!(0.5));

        // No context snapshot yet: usage only.
        let live = parse_metrics(
            &json!({"complete": true, "totals": {"turns": 0}, "by_session": [{"session_id": "s"}]}),
            "s",
        );
        assert_eq!((live.model, live.last_generate_at), (None, None));
        assert_eq!(parse_metrics(&Value::Null, "s"), LiveNode::default());

        assert_eq!(
            parse_harness_status(&json!({"status": "completed", "expects_wake": true})),
            (Some("completed".to_string()), Some(true))
        );
        assert_eq!(parse_harness_status(&Value::Null), (None, None));
    }

    #[test]
    fn live_targets_skip_nodes_whose_send_never_landed() {
        let rec = record(json!({
            "plan": {"state": "done", "session_id": "wf_r_1_plan@r0", "pending_at": 5},
            "read#0": {"state": "running", "session_id": "wf_r_1_read#0@r1", "pending_at": 7},
            "read#1": {"state": "failed", "session_id": "wf_r_1_read#1@r0",
                       "result_error": "send refused"},
            "read#2": {"state": "pending", "session_id": "wf_r_1_read#2@r0"}
        }));
        let targets = live_targets(&rec, None);
        assert_eq!(
            targets,
            vec![
                ("read#0".to_string(), "wf_r_1_read#0@r1".to_string(), true),
                ("plan".to_string(), "wf_r_1_plan@r0".to_string(), false),
            ]
        );
    }

    #[test]
    fn profile_node_model_omitted_until_observed() {
        let mut rec = record(json!({"read#0": {"state": "running", "session_id": "s"}}));
        rec.fanout_src.insert("read".into(), vec![json!(0)]);
        let resp = build_status(rec.clone(), Some(&def()), &BTreeMap::new(), true);
        let read = &resp.node_details["read#0"];
        assert_eq!(read.profile.as_deref(), Some("reader"));
        assert_eq!(read.model, None, "the profile may pin another model");

        let live = BTreeMap::from([(
            "read#0".to_string(),
            LiveNode {
                model: Some("pinned".into()),
                harness_status: Some("running".into()),
                expects_wake: Some(false),
                ..Default::default()
            },
        )]);
        let resp = build_status(rec, Some(&def()), &live, true);
        let read = &resp.node_details["read#0"];
        assert_eq!(read.model.as_deref(), Some("pinned"));
        assert_eq!(read.harness_status.as_deref(), Some("running"));
    }
}
