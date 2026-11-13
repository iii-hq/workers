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

/// What a node's failure does to the rest of the run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnError {
    /// A failed or cancelled node fails the run when the output needs it.
    #[default]
    Fail,
    /// A failed or cancelled node counts as finished with a `null` result: its
    /// consumers still fire, and a fan-in keeps the `null` at its position.
    Continue,
}

impl OnError {
    pub fn is_fail(&self) -> bool {
        *self == Self::Fail
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    #[default]
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
    /// Which node's result (or nodes' results) the whole run returns.
    pub output: OutputRef,
    /// Dispatch policy inherited by every node that omits `agent.functions` (same
    /// shape); when unset those nodes get everything except `workflow::*`,
    /// `configuration::*`, `approval::*`, `harness::hook::*` and `harness::send`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_functions: Option<Value>,
    /// At most this many node sessions live at once (running, or reserved for a
    /// send); further ready nodes wait for a slot. At least 1; unset means no cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
}

fn default_def_version() -> u32 {
    1
}

/// Selects the node(s) whose result becomes the run's `result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    pub from: OutputFrom,
}

/// The run's result: one node's result, or several joined into one object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OutputFrom {
    /// `"node:<id>"` of an existing node; the run returns its result as is. For a
    /// fanout group the result is the array of its children's results, in order.
    // A bare `<id>` is also accepted.
    One(String),
    /// Several `"node:<id>"` refs; the run returns an object keyed by node id, so
    /// independent nodes (text ones too) can all be required.
    Many(Vec<String>),
}

impl OutputFrom {
    /// The referenced node ids, `node:` stripped (a bare id is kept as is).
    pub fn node_ids(&self) -> Vec<&str> {
        let refs: &[String] = match self {
            OutputFrom::One(s) => std::slice::from_ref(s),
            OutputFrom::Many(v) => v,
        };
        refs.iter()
            .map(|s| s.strip_prefix("node:").unwrap_or(s))
            .collect()
    }
}

impl From<&str> for OutputFrom {
    fn from(s: &str) -> Self {
        OutputFrom::One(s.to_string())
    }
}

/// One agent in the DAG, plus its wiring (inputs, dependency edges, optional
/// fan-out).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// Milliseconds an attempt may run, wall clock from its send (harness queue wait
    /// and parked time included; checked once a minute). At least 1000; defaults to
    /// the worker's `default_pending_timeout_ms`. Fanout items use the node's value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Refires after a timeout, each in a new session; a failed turn is never
    /// retried. At most 10; the default refires once, and a refire repeats the
    /// node's side effects. `0` fails the node at its first timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    /// `"fail"` (default): a failed or cancelled node fails the run when the output
    /// needs it. `"continue"`: it counts as finished with a `null` result, so
    /// consumers still fire and the run can complete; the error stays in
    /// `node_errors`. Retries run first; a run abort still cancels everything.
    #[serde(default, skip_serializing_if = "OnError::is_fail")]
    pub on_error: OnError,
    /// Display name, at most 80 characters: the node session's title (a fanout item
    /// gets ` #<i>`) and the node's entry in `workflow::status` details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Group name, at most 80 characters; `workflow::status` counts the nodes of
    /// each phase (total, done, failed, running).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Bounded do-while loop carried by this node (the loop node): this node and
    /// its `body` run again, as new sessions, while `while` holds on this node's
    /// result. Outside the loop only this node can be read, and readers get its
    /// last pass.
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub loop_spec: Option<LoopSpec>,
}

/// A bounded do-while loop on the loop node. Pass `k` of a node runs as uid
/// `<id>#it<k>` (the first pass keeps `<id>`); every node of the loop gets
/// `{"input": <its input>, "loop_previous": <the loop node's previous result>}`, and
/// a body node also `"own_previous": <its own previous result>`, both null on the
/// first pass: the same form as a fanout copy's `{"input": …, "fanout_item": …}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopSpec {
    /// Other node ids that repeat with the loop node; the loop node must read each
    /// of them, directly or through other body nodes. No loop node may declare
    /// `fanout`.
    #[serde(default)]
    pub body: Vec<String>,
    /// The loop repeats while this condition holds on the loop node's JSON result.
    #[serde(rename = "while")]
    pub cond: LoopCondition,
    /// Passes after the first, 1 to 9: `2` means at most 3 passes. The loop also
    /// ends quietly at the run's materialized-node cap.
    pub max_repeats: u32,
}

/// `field` of the loop node's result equals `equals`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopCondition {
    /// Dotted path into the loop node's JSON result, e.g. `"verdict"`. The loop
    /// node's `schema` must list each segment in `required`.
    pub field: String,
    /// The loop repeats while the field is this string; a missing field, a
    /// non-string value or a null result does not hold.
    pub equals: String,
}

/// The sub-agent that runs a node (one harness session).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    /// Model id, resolved at start: as written (and checked against
    /// `router::models::list`), else the caller's model. Empty only on a profile node
    /// whose caller model was unreadable or that wrote a `provider`, so the profile's
    /// pinned model applies.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// Agent profile id (`directory::agents::*`: lowercase letters, digits, `-` and
    /// `_`, at most 64 characters), sent as the harness `options.agent`.
    /// The profile supplies the system prompt, so `system_prompt` must be omitted. A
    /// model the profile pins (also through `extends`) wins over `model`. The profile
    /// grants no permissions: `functions` still decides them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
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
    /// Output contract; a node another node fans out over must be
    /// `{"type":"json","schema":{...}}` declaring every field read downstream. A node
    /// read by `input.from` may be text: its consumer gets the whole text as a JSON
    /// string.
    // `schema` must be a real JSON Schema with a top-level `type`; an empty `{}` is
    // rejected at `workflow::start`. An undeclared field tends not to be emitted, so a
    // fanout over it fails at runtime. Omit (or `{ "type": "text" }`) for a text node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// Absolute folder the node's session runs in, in place of the caller's folder.
    /// It must lie inside the caller's folder (symlinks resolved), which is checked
    /// once, at start; a run without a caller folder cannot set it. Fanout items
    /// and loop passes share it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_root: Option<String>,
}

impl AgentSpec {
    /// The output contract when it is JSON (`{"type":"json","schema"?}`).
    pub fn json_output(&self) -> Option<&Value> {
        self.output
            .as_ref()
            .filter(|output| output.get("type").and_then(Value::as_str) == Some("json"))
    }
}

/// Where a node's input comes from: one source, or an array of `"node:<id>"` refs
/// gathered into one object keyed by node id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InputFrom {
    /// `"run_input"` (the run's top-level input), `"node:<id>"` (that dependency's
    /// result; a text node's result arrives as a JSON string), or `"fanout_item"`
    /// (this fanout child's array element).
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

impl Default for InputSpec {
    fn default() -> Self {
        Self {
            from: "run_input".into(),
            template: None,
        }
    }
}

/// Per-item parallelism: expand a node into one child per element of an array.
/// A child whose `input.from` is `"fanout_item"` receives its element; any other
/// source gives it `{"input": <the gathered input>, "fanout_item": <its element>}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FanoutSpec {
    /// `"node:<dep>.<path>"` to an array the dependency's `agent.output` schema declares;
    /// one child runs per element. Empty when `items` is set.
    // An undeclared path fails at expansion time naming what the dep DID produce.
    // The group's result is the array of child results, in order.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub over: String,
    /// The array written in the request; when set, it is the array and `over` is
    /// not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<Value>>,
}

/// Completion callback: `function_id` is called once on terminal state with
/// `{run_id, status, result, result_error, failed_nodes}`; at-least-once, dedup on `run_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotifySpec {
    /// Function id to call on terminal state, e.g. `"myworker::wf-done"`.
    pub function_id: String,
    /// Queue for durable delivery; defaults to the worker's `"workflow-notify"` queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
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
    /// Session that started this run (the chat that called `workflow::start`).
    /// Stamped onto every node session's metadata as `parent_session_id` so the
    /// console nests workflow nodes under their orchestrator. `None` for a
    /// non-agent caller (no session to nest under).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_session_id: Option<String>,
    /// Caller's folder, stamped by the pre-call hook from the caller turn's
    /// `metadata.fs_scope.root`. Every node send carries it as
    /// `options.metadata.fs_scope.root`; `None` leaves node sessions in the
    /// harness default folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_fs_root: Option<String>,
    /// Scoped idempotency key that points at this run; the GC deletes it with the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idem_key: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    #[serde(default)]
    pub retries: u32,
    /// Corrections reconcile sent into this attempt's session after a JSON
    /// node's settled answer did not parse or match its schema.
    #[serde(default)]
    pub corrections: u32,
    /// The node delivered its result with `workflow-result::submit` in this
    /// attempt; the value is stored as the node's result.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub submitted: bool,
    /// First failed `harness::send` of this attempt; the send is retried until
    /// the node's timeout has passed since then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_failed_at: Option<i64>,
    /// When the node reached Done, Failed or Cancelled (stamped on the next write).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

impl NodeCheckpoint {
    /// Running, or Pending with a reserved session (a send may have reached the harness).
    pub fn is_live(&self) -> bool {
        self.state == NodeState::Running
            || (self.state == NodeState::Pending && self.session_id.is_some())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn fanout_spec_reads_old_over_and_round_trips_items() {
        let old: FanoutSpec = serde_json::from_value(json!({"over": "node:a.items"})).unwrap();
        assert_eq!(old.over, "node:a.items");
        assert_eq!(old.items, None);
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            json!({"over": "node:a.items"})
        );

        let items: FanoutSpec = serde_json::from_value(json!({"items": [1, 2]})).unwrap();
        assert!(items.over.is_empty());
        assert_eq!(items.items, Some(vec![json!(1), json!(2)]));
        assert_eq!(
            serde_json::to_value(&items).unwrap(),
            json!({"items": [1, 2]})
        );
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
                        ..Default::default()
                    },
                );
                m
            },
            fanout_src: BTreeMap::new(),
            result: None,
            result_error: None,
            notify: None,
            caller_session_id: None,
            caller_fs_root: Some("/home/u/proj".to_string()),
            idem_key: None,
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
        assert!(
            record.caller_fs_root.is_none(),
            "caller_fs_root should default to None"
        );
    }

    #[test]
    fn old_definition_json_still_deserializes() {
        let def: WorkflowDef = serde_json::from_value(json!({
            "nodes": {"a": {"agent": {"model": "m"}, "input": {"from": "run_input"}}},
            "output": {"from": "node:a"}
        }))
        .expect("deserialize old definition");
        assert_eq!(def.max_concurrency, None);
        assert_eq!(def.nodes["a"].timeout_ms, None);
        assert_eq!(def.nodes["a"].retries, None);
        assert_eq!(def.nodes["a"].loop_spec, None);
        let out = serde_json::to_value(&def).unwrap();
        assert!(out["nodes"]["a"].get("loop").is_none(), "{out}");
    }

    #[test]
    fn loop_spec_round_trips_and_rejects_unknown_keys() {
        let node = json!({
            "agent": {"model": "m"},
            "input": {"from": "node:w"},
            "loop": {"body": ["w"], "while": {"field": "verdict", "equals": "reject"}, "max_repeats": 2}
        });
        let def: NodeDef = serde_json::from_value(node.clone()).expect("deserialize loop");
        let spec = def.loop_spec.as_ref().expect("loop parsed");
        assert_eq!(spec.body, vec!["w".to_string()]);
        assert_eq!(spec.cond.field, "verdict");
        assert_eq!(spec.cond.equals, "reject");
        assert_eq!(spec.max_repeats, 2);
        let out = serde_json::to_value(&def).unwrap();
        assert_eq!(out["loop"], node["loop"]);

        let body_default: LoopSpec = serde_json::from_value(
            json!({"while": {"field": "v", "equals": "x"}, "max_repeats": 1}),
        )
        .unwrap();
        assert!(body_default.body.is_empty());

        for bad in [
            json!({"while": {"field": "v", "equals": "x"}, "max_repeats": 1, "until": 1}),
            json!({"while": {"field": "v", "equals": "x", "not": true}, "max_repeats": 1}),
        ] {
            assert!(
                serde_json::from_value::<LoopSpec>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn old_checkpoint_with_pending_timeout_ms_deserializes() {
        let cp: NodeCheckpoint = serde_json::from_value(json!({
            "state": "running",
            "session_id": "wf_r_a",
            "pending_at": 5,
            "pending_timeout_ms": 60_000,
            "retries": 1
        }))
        .expect("deserialize old checkpoint");
        assert_eq!(cp.state, NodeState::Running);
        assert_eq!(cp.retries, 1);
        assert!(serde_json::to_value(&cp)
            .unwrap()
            .get("pending_timeout_ms")
            .is_none());
    }

    #[test]
    fn definition_with_node_options_round_trips() {
        let v = json!({
            "version": 1,
            "nodes": {"a": {
                "agent": {"model": "m"},
                "input": {"from": "run_input"},
                "depends_on": [],
                "timeout_ms": 120_000,
                "retries": 0
            }},
            "output": {"from": "node:a"},
            "max_concurrency": 3
        });
        let def: WorkflowDef = serde_json::from_value(v.clone()).expect("deserialize");
        assert_eq!(def.max_concurrency, Some(3));
        assert_eq!(def.nodes["a"].timeout_ms, Some(120_000));
        assert_eq!(def.nodes["a"].retries, Some(0));
        assert_eq!(serde_json::to_value(&def).unwrap(), v);
    }

    #[test]
    fn run_status_serializes_snake_case() {
        let awaiting = serde_json::to_value(RunStatus::AwaitingNodes).expect("serialize");
        assert_eq!(awaiting, json!("awaiting_nodes"));

        let done = serde_json::to_value(NodeState::Done).expect("serialize");
        assert_eq!(done, json!("done"));
    }
}
