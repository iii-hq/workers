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

/// A declarative multi-agent DAG. Nodes are sub-agents, `depends_on` are the
/// edges, `fanout` runs a node once per item of an upstream array (with a
/// `depends_on` join acting as the barrier), and `output` picks which node's
/// result the run returns. Each node runs as its own harness session, so a run
/// is durable and crash-resumable. Submit via `workflow::start`, which returns
/// a run_id immediately; supply `notify` to be pushed the outcome when the run
/// finishes, or poll `workflow::status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDef {
    /// Schema version. Defaults to `1` (the only supported version) when omitted,
    /// so callers never need to send it.
    #[serde(default = "default_def_version")]
    pub version: u32,
    /// Nodes keyed by node id. A node id must not contain `#`, `.`, or `/` (reserved
    /// for fanout-item / over-path parsing and result-key composition). The graph
    /// formed by every node's `depends_on` must be acyclic.
    pub nodes: BTreeMap<String, NodeDef>,
    /// Which node's result the whole run returns.
    pub output: OutputRef,
    /// Default dispatch policy for every node that omits its own
    /// `agent.functions` — the run-wide reach inherited by un-narrowed nodes.
    /// Same shape as `agent.functions` (a bare allow-list array, a single
    /// string, or a `{ allow?, deny }` object). Omit and un-narrowed nodes get a
    /// deny-only policy that permits everything outside the workflow
    /// control-plane namespaces. Set this to mirror a narrower main-agent
    /// policy run-wide (for example `{ "deny": ["approval::*"] }`), then narrow
    /// individual nodes further with their own `agent.functions`.
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
    /// `"node:<id>"` (a bare `<id>` is also accepted) of an existing node. If
    /// that node is a fanout group, the run's result is the array of its
    /// children's results, in order.
    pub from: String,
}

/// One agent in the DAG, plus its wiring (inputs, dependency edges, optional
/// fan-out).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    pub agent: AgentSpec,
    pub input: InputSpec,
    /// Prerequisite node ids. This node fires only once ALL of them are Done —
    /// this is the barrier / join. Empty means it can start immediately.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// When set, this node fans out: one child agent runs per item of the
    /// referenced array. Omit for a normal single-run node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout: Option<FanoutSpec>,
}

/// The sub-agent that runs a node (one harness session).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    /// Model id, e.g. `"claude-sonnet-4-6"`. Required and non-empty. MUST be a
    /// model registered on this engine — discover the valid ids with
    /// `router::models::list`; an unregistered id is rejected up front by
    /// `workflow::start`. Do not guess model ids from memory.
    pub model: String,
    /// Optional provider override (e.g. `"anthropic"`); defaults to the routed
    /// provider for the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Optional system prompt; appended to the child's built-in identity prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Allow-list of function ids the child may call, e.g. `["web::fetch"]`. A
    /// bare array is the shorthand and is wrapped into the harness
    /// `{ "allow": [...] }` policy; a single string and a full FunctionPolicy
    /// object (`{ "allow": [...], "deny": [...] }`) are also accepted.
    /// INHERITS BY DEFAULT: omit this and the node inherits the run's default
    /// reach — the def-level `default_functions`, or the implicit deny-only
    /// workflow policy when that is also unset. So a node can call anything
    /// outside the workflow control plane without listing it. Set this to
    /// NARROW one node (e.g. `["web::fetch"]`), or to an empty allow-list
    /// (`{ "allow": [] }`) to lock the node down to nothing. Use the narrow
    /// form for least-privilege nodes; omit for the common case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<Value>,
    /// Output contract passed to the harness. MUST be `{ "type": "json",
    /// "schema": { ... } }` on any node that another node reads via
    /// `input.from = "node:<id>"` or fans out over (a consumed node has to emit
    /// JSON). The `schema` MUST declare every field a downstream node reaches —
    /// each `fanout.over` path and each dotted `input.from` path. If some node
    /// has `fanout.over = "node:X.items"`, node X's schema must declare `items`
    /// as an array; an undeclared field tends not to be emitted and the fanout
    /// fails at runtime. The `schema` must be a real JSON Schema with a top-level
    /// `type` (e.g. `"object"`); an empty `{}` is rejected at `workflow::start`.
    /// The final `output` node may be plain text — omit this, or set
    /// `{ "type": "text" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
}

/// Where a node's input value comes from. Either a single source, or — for a
/// join/synthesis node that must read MORE THAN ONE dependency — an array of
/// `"node:<id>"` references that are gathered into one keyed object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InputFrom {
    /// A single source:
    /// - `"run_input"` / `"workflow.input"` — the run's top-level `input`.
    /// - `"node:<id>"` — that dependency's JSON result.
    /// - `"fanout_item"` — on a fanout child, its single array element.
    One(String),
    /// Several `"node:<id>"` sources joined into one object: each entry becomes a
    /// field keyed by its node id (e.g. `["node:write","node:reviews"]` →
    /// `{ "write": <write result>, "reviews": <reviews result> }`). A fanout dep
    /// resolves to its array of child results. Every entry MUST be a `"node:<id>"`
    /// reference and MUST appear in the node's `depends_on`.
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
    /// Source of the input value — a single source or, for a join node, an array
    /// of `"node:<id>"` references gathered into a keyed object. See `InputFrom`.
    pub from: InputFrom,
    /// Optional text prepended to the (JSON-serialized) input as the opening
    /// message — use it to instruct the agent what to do with the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// Per-item parallelism: expand a node into one child per element of an array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FanoutSpec {
    /// `"node:<dep_id>.<path.to.array>"` — points at an array inside a
    /// dependency's JSON result (the dep must declare JSON output). The fanout
    /// path must name a field the dependency's `agent.output` schema actually
    /// declares and emits, otherwise the run fails at expansion time with a
    /// message naming what the dep DID produce. One child runs per element, each
    /// reading its element via `input.from = "fanout_item"`. The group's result
    /// is the array of child results, in order.
    pub over: String,
}

/// Caller-supplied completion callback. When a run reaches a terminal state the
/// worker triggers `function_id` once with `{run_id, status, result,
/// result_error}` so the caller is pushed the outcome instead of polling
/// `workflow::status`. Delivery is durable (enqueued) and at-least-once — the
/// handler must dedup on `run_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotifySpec {
    /// Function id to trigger on terminal state, e.g. `"myworker::wf-done"`.
    pub function_id: String,
    /// Queue for durable delivery; defaults to `"default"` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
}

/// Deliver the run outcome into a session as a new message on terminal state
/// (via `harness::send`), so the calling agent never has to poll
/// `workflow::status`. An AGENT caller sends `reply_to: {}` (optionally a
/// `template`) and leaves the rest unset: the worker's pre_trigger hook
/// auto-stamps the caller's real `session_id` / `model` / `provider`, OVERWRITING
/// anything supplied — so an agent cannot direct a run's result into another
/// session. A trusted worker caller (not going through the agent turn loop) may
/// set these explicitly.
///
/// After calling `workflow::start` with `reply_to`, END YOUR TURN — do NOT claim
/// the result was delivered and do NOT produce or guess a result this turn. The
/// outcome arrives as a SEPARATE message when the run finishes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplySpec {
    /// Target session. Auto-stamped from the caller's turn by the pre_trigger hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Model for the reply turn — `harness::send` requires one and does NOT
    /// inherit the session's model. Auto-stamped from the caller's turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional provider override, auto-stamped from the caller's turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Optional text prepended to the formatted outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Caller turn's dispatch policy, auto-stamped by the pre_trigger hook so
    /// the reply can wake an idle caller with its exact original reach. When
    /// absent, delivery omits `options.functions` and the harness resolves its
    /// configured default. A `FunctionPolicy` object (`{allow?, deny}`), carried
    /// as a `Value` since the worker only passes it through to `harness::send`.
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
            "allow-list",                        // AgentSpec.functions shorthand
            "barrier",                           // depends_on join semantics
            "crash-resumable",                   // WorkflowDef top-level behavior
            "must not contain",                  // node-id constraint
            "consumed node has to emit json",    // AgentSpec.output JSON-output rule
            "router::models::list",              // AgentSpec.model discovery hint
            "fanout path must name a field",     // FanoutSpec.over: dep schema must declare+emit it
            "real json schema with a top-level", // AgentSpec.output: no empty {} schema
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
