use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    AwaitingNodes,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

// ---------------------------------------------------------------------------
// Workflow definition types
// ---------------------------------------------------------------------------

/// Declarative multi-agent DAG, durable and crash-resumable: nodes are sub-agent
/// sessions, `depends_on` the edges, `output` the returned node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDef {
    /// Schema version; only `1` is supported.
    #[serde(default = "default_def_version")]
    pub version: u32,
    /// Nodes keyed by node id; an id must not contain `#`, `.`, or `/`, and the
    /// `depends_on` graph must be acyclic.
    // `#` / `.` / `/` are reserved for fanout-item / over-path parsing and result-key composition.
    pub nodes: BTreeMap<String, NodeDef>,
    /// Which node's result the whole run returns.
    pub output: OutputRef,
    /// Dispatch policy inherited by every node that omits `agent.functions` (same
    /// shape); when unset those nodes get `["*"]`, everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_functions: Option<Value>,
}

fn default_def_version() -> u32 {
    1
}

/// Selects the node whose result becomes the run's `result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    /// `"node:<id>"` of an existing node; for a fanout group the result is the
    /// array of its children's results, in order.
    // A bare `<id>` is also accepted.
    pub from: String,
}

/// One agent in the DAG, plus its wiring (inputs, dependency edges, optional
/// fan-out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    pub agent: AgentSpec,
    pub input: InputSpec,
    /// Prerequisite node ids; the node fires only once all of them are Done (the
    /// barrier / join).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Fan out: one child agent runs per item of the referenced array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<FanoutSpec>,
}

/// The sub-agent that runs a node (one harness session).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    /// Model id registered on this engine (discover them with `router::models::list`);
    /// an unregistered id is rejected at start.
    pub model: String,
    /// Provider override; defaults to the routed provider for the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// System prompt appended to the child's built-in identity prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Child dispatch policy: an allow-list array, a single string, or
    /// `{ "allow": [...], "deny": [...] }`; omit to inherit `default_functions`.
    // A bare array is wrapped into the harness `{ "allow": [...] }` FunctionPolicy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<Value>,
    /// Output contract; any node another node reads or fans out over must be
    /// `{"type":"json","schema":{...}}` declaring every field read downstream.
    // `schema` must be a real JSON Schema with a top-level `type`; an empty `{}` is
    // rejected at `workflow::start`. An undeclared field tends not to be emitted, so a
    // fanout over it fails at runtime. Omit (or `{ "type": "text" }`) for a text node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

/// Where a node's input comes from: one source, or an array of `"node:<id>"` refs
/// gathered into one object keyed by node id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InputFrom {
    /// `"run_input"` (the run's top-level input), `"node:<id>"` (that dependency's
    /// result), or `"fanout_item"` (this fanout child's array element).
    // `"workflow.input"` is accepted as an alias of `"run_input"`.
    One(String),
    /// Several `"node:<id>"` sources, each also listed in `depends_on`, joined into
    /// one object keyed by node id.
    // A fanout dep resolves to its array of child results.
    Many(Vec<String>),
}

impl InputFrom {
    /// True iff this is the single literal source `lit` (e.g. `"fanout_item"`).
    pub fn is_literal(&self, lit: &str) -> bool {
        matches!(self, InputFrom::One(s) if s == lit)
    }

    /// All source strings, whether one or many — for uniform validation.
    pub fn sources(&self) -> &[String] {
        match self {
            InputFrom::One(s) => std::slice::from_ref(s),
            InputFrom::Many(v) => v.as_slice(),
        }
    }
}

impl From<&str> for InputFrom {
    fn from(s: &str) -> Self {
        InputFrom::One(s.to_string())
    }
}

/// What a node receives as its opening message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    /// Source of the input value: a single source or, for a join node, an array of
    /// `"node:<id>"` refs.
    pub from: InputFrom,
    /// Text prepended to the JSON-serialized input as the node's opening message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// Per-item parallelism: expand a node into one child per element of an array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FanoutSpec {
    /// `"node:<dep>.<path>"` to an array the dependency's `agent.output` schema declares;
    /// one child runs per element, reading it as `"fanout_item"`.
    // An undeclared path fails at expansion time naming what the dep DID produce.
    // The group's result is the array of child results, in order.
    pub over: String,
}

/// Completion callback: `function_id` is triggered once on terminal state with
/// `{run_id, status, result, result_error}`; at-least-once, dedup on `run_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotifySpec {
    /// Function id to trigger on terminal state, e.g. `"myworker::wf-done"`.
    pub function_id: String,
    /// Queue for durable delivery; defaults to `"default"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

/// Push the outcome into the caller's session as a message; agents send `{}`
/// (optionally `template`) — the rest is auto-stamped from the caller's turn.
// Delivered via `harness::send`. The `workflow::stamp-reply` pre_trigger hook OVERWRITES
// any supplied session_id/model/provider, so an agent cannot direct a result into another
// session; a trusted worker caller (outside the agent turn loop) may set them explicitly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplySpec {
    /// Target session; auto-stamped from the caller's turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Model for the reply turn; auto-stamped from the caller's turn.
    // `harness::send` requires one and does NOT inherit the session's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Provider override; auto-stamped from the caller's turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Text prepended to the formatted outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Caller turn's dispatch policy (`{allow, deny}`), auto-stamped so the reply can
    /// wake an idle caller with its original reach.
    // Present → `harness::send` with `run: true`; absent → passive transcript append
    // (`run: false`). Carried as a `Value`: the worker only passes it through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<Value>,
}

// ---------------------------------------------------------------------------
// Run record types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowRunRecord {
    pub run_id: String,
    /// Monotonic dequeue guard
    pub step: u64,
    pub status: RunStatus,
    /// Observed by tick → finalize Cancelled
    #[serde(default)]
    pub abort: bool,
    /// Key into workflow_def/<run_id>
    pub def_ref: String,
    pub input: Value,
    /// Keyed by node_uid
    #[serde(default)]
    pub nodes: BTreeMap<String, NodeCheckpoint>,
    /// node_id → FROZEN `over` snapshot; N = len
    #[serde(default)]
    pub fanout_src: BTreeMap<String, Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    /// Caller-supplied completion callback (push instead of poll). See `NotifySpec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<NotifySpec>,
    /// Caller-supplied "reply into my session" delivery: push the outcome as a
    /// session message instead of a function callback. See `ReplySpec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplySpec>,
    /// Session that started this run (the chat that called `workflow::start`).
    /// Stamped onto every node session's metadata as `parent_session_id` so the
    /// console nests workflow nodes under their orchestrator. `None` for a
    /// non-agent caller (no session to nest under).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NodeCheckpoint {
    pub state: NodeState,
    /// Deterministic: wf_<run_id>_<node_uid>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The turn we fired; reconcile MUST match this
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Key into workflow_node_result/<run_id>/<node_uid>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ref: Option<String>,
    /// Set when a 'completed' turn carried result_error
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_timeout_ms: Option<u64>,
    #[serde(default)]
    pub retries: u32,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// The whole point of the doc comments on the definition types: `engine::
    /// functions::info` must serve a self-documenting WorkflowDef schema so an
    /// agent can author a run from the LIVE engine without loading any skill.
    /// schemars surfaces `///` doc comments as `description`; this asserts the
    /// load-bearing semantics survive into the generated schema.
    #[test]
    fn workflow_def_schema_is_self_documenting() {
        let schema = schemars::schema_for!(WorkflowDef);
        let blob = serde_json::to_string(&schema)
            .expect("serialize schema")
            .to_lowercase();
        for needle in [
            "run_input", // InputSpec.from sources
            "fanout_item",
            "node:<id>",
            "allow-list",                            // AgentSpec.functions shorthand
            "barrier",                               // depends_on join semantics
            "crash-resumable",                       // WorkflowDef top-level behavior
            "must not contain",                      // node-id constraint
            "fans out over must be",                 // AgentSpec.output JSON-output rule
            "router::models::list",                  // AgentSpec.model discovery hint
            "one child runs per element",            // FanoutSpec.over expansion semantics
            "declaring every field read downstream", // AgentSpec.output: schema must declare consumed fields
        ] {
            assert!(
                blob.contains(needle),
                "WorkflowDef schema is missing the semantic doc fragment {:?} — the live \
                 engine would no longer be self-describing for that rule",
                needle
            );
        }
    }

    #[test]
    fn run_status_is_terminal() {
        assert!(RunStatus::Completed.is_terminal());
        assert!(RunStatus::Failed.is_terminal());
        assert!(RunStatus::Cancelled.is_terminal());
        assert!(!RunStatus::Running.is_terminal());
        assert!(!RunStatus::AwaitingNodes.is_terminal());
    }

    #[test]
    fn record_round_trips_through_json() {
        let record = WorkflowRunRecord {
            run_id: "run_abc123".to_string(),
            step: 3,
            status: RunStatus::AwaitingNodes,
            abort: false,
            def_ref: "run_abc123".to_string(),
            input: json!({"topic": "test"}),
            nodes: {
                let mut m = BTreeMap::new();
                m.insert(
                    "plan".to_string(),
                    NodeCheckpoint {
                        state: NodeState::Done,
                        session_id: Some("wf_run_abc123_plan".to_string()),
                        turn_id: Some("turn_xyz".to_string()),
                        result_ref: Some("run_abc123/plan".to_string()),
                        result_error: None,
                        pending_at: None,
                        pending_timeout_ms: None,
                        retries: 0,
                    },
                );
                m
            },
            fanout_src: BTreeMap::new(),
            result: None,
            result_error: None,
            notify: None,
            reply_to: None,
            caller_session_id: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_001,
        };

        let json_str = serde_json::to_string(&record).expect("serialize");
        let decoded: WorkflowRunRecord = serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(record, decoded);
    }

    #[test]
    fn defaults_fill_in_for_a_minimal_record() {
        let v: Value = json!({
            "run_id": "run_min",
            "step": 0,
            "status": "running",
            "def_ref": "run_min",
            "input": {},
            "created_at": 1_700_000_000_i64,
            "updated_at": 1_700_000_000_i64
        });

        let record: WorkflowRunRecord = serde_json::from_value(v).expect("deserialize minimal");

        assert!(!record.abort, "abort should default to false");
        assert!(record.nodes.is_empty(), "nodes should default to empty");
        assert!(
            record.fanout_src.is_empty(),
            "fanout_src should default to empty"
        );
        assert!(record.result.is_none(), "result should default to None");
        assert!(
            record.result_error.is_none(),
            "result_error should default to None"
        );
    }

    #[test]
    fn run_status_serializes_snake_case() {
        let awaiting = serde_json::to_value(RunStatus::AwaitingNodes).expect("serialize");
        assert_eq!(awaiting, json!("awaiting_nodes"));

        let done = serde_json::to_value(NodeState::Done).expect("serialize");
        assert_eq!(done, json!("done"));
    }
}
