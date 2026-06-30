use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    error::WorkflowError,
    ids::new_run_id,
    state,
    types::{RunStatus, WorkflowDef, WorkflowRunRecord},
};

use super::Deps;

// Guardrails on an untrusted definition.
// ponytail: generous caps to stop runaway/DoS inputs; tighten if abused.
const MAX_NODES: usize = 10_000;
const MAX_IDEM_KEY_LEN: usize = 1024;
const SUPPORTED_DEF_VERSION: u32 = 1;
// Cap on sub-workflow nesting. The default node policy denies `workflow::*`, but a
// node can opt back in with an explicit `functions`, so a node could launch a
// sub-workflow whose node launches another, unbounded. Bound the chain. Safety
// ceiling, not a tuning knob.
const MAX_WORKFLOW_DEPTH: usize = 8;

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, JsonSchema)]
pub struct StartRequest {
    /// The workflow DAG to run. See `WorkflowDef`. (`workflow` is accepted as an
    /// alias for the wrapper key.)
    pub definition: WorkflowDef,
    /// Top-level input made available to nodes whose `input.from` is
    /// `"run_input"` / `"workflow.input"`. Any JSON value.
    #[serde(default)]
    pub input: Value,
    /// Optional dedupe key: a repeated key returns the original `run_id` instead
    /// of launching a duplicate run.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Optional completion callback: a function the worker triggers once when the
    /// run reaches a terminal state, so the caller is pushed the outcome instead
    /// of polling `workflow::status`.
    #[serde(default)]
    pub notify: Option<crate::types::NotifySpec>,
    /// Optional: deliver the run outcome into a session as a new message on
    /// terminal state (via `harness::send`), so an agent caller never polls
    /// `workflow::status`. Send `reply_to: {}` and the worker auto-stamps your
    /// session/model; add a `template` to prefix the message. See `ReplySpec`.
    ///
    /// After calling with `reply_to` (or `notify`), END YOUR TURN — do NOT claim
    /// the result was delivered or produce one this turn; it arrives as a separate
    /// message when the run finishes.
    #[serde(default)]
    pub reply_to: Option<crate::types::ReplySpec>,
    /// The orchestrator session, stamped by the `workflow::stamp-reply`
    /// pre_trigger hook from the caller's turn (never trusted from the agent).
    /// Recorded on the run so node sessions can nest under it in the console.
    #[serde(default)]
    pub caller_session_id: Option<String>,
}

/// Internal mirror used for the typed parse AFTER `StartRequest`'s custom
/// `Deserialize` has normalized the payload and collected structural problems.
#[derive(Deserialize)]
struct StartRequestRaw {
    #[serde(alias = "workflow")]
    definition: WorkflowDef,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    notify: Option<crate::types::NotifySpec>,
    #[serde(default)]
    reply_to: Option<crate::types::ReplySpec>,
    #[serde(default)]
    caller_session_id: Option<String>,
}

/// Compact copy-pasteable skeleton appended to every shape error.
const SHAPE_HINT: &str = "Expected shape: \
    {\"definition\":{\"nodes\":{\"<id>\":{\"agent\":{\"model\":\"<id from router::models::list>\"},\
    \"input\":{\"from\":\"run_input\"}}},\"output\":{\"from\":\"node:<id>\"}}}. `version` defaults to 1. \
    Each node is {agent, input, depends_on?, fanout?}; a pure source node may omit `input` (defaults to \
    run_input). Full field docs are inline in this function's request schema.";

const ALLOWED_DEF_KEYS: &[&str] = &["version", "nodes", "output", "default_functions"];
const ALLOWED_NODE_KEYS: &[&str] = &["agent", "input", "depends_on", "fanout"];
const ALLOWED_AGENT_KEYS: &[&str] = &["model", "provider", "system_prompt", "functions", "output"];

// Custom Deserialize so a malformed `definition` yields ONE error listing EVERY
// structural problem (plus the canonical shape), instead of serde's fail-fast
// one-field-at-a-time errors that make a weak model play whack-a-mole. It also
// applies the `workflow` alias and the source-node `input` default. The typed
// `StartRequestRaw` parse and `validate_def` (semantic rules) run afterward.
impl<'de> Deserialize<'de> for StartRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let mut v = Value::deserialize(deserializer)?;
        normalize_request(&mut v);
        let problems = collect_def_problems(&v);
        if !problems.is_empty() {
            return Err(D::Error::custom(format_problems(&problems)));
        }
        serde_json::from_value::<StartRequestRaw>(v)
            .map(|r| StartRequest {
                definition: r.definition,
                input: r.input,
                idempotency_key: r.idempotency_key,
                notify: r.notify,
                reply_to: r.reply_to,
                caller_session_id: r.caller_session_id,
            })
            .map_err(|e| D::Error::custom(format!("{e}. {SHAPE_HINT}")))
    }
}

/// Accept the `workflow` wrapper alias and inject `input:{from:"run_input"}` into
/// pure-source nodes (no `depends_on`, no `fanout`, no `input`) — the most common
/// omission. Non-source nodes are left alone so a missing `input` is REPORTED,
/// never silently mis-wired.
fn normalize_request(v: &mut Value) {
    let Some(obj) = v.as_object_mut() else {
        return;
    };
    if !obj.contains_key("definition") {
        if let Some(w) = obj.remove("workflow") {
            obj.insert("definition".into(), w);
        }
    }
    let Some(nodes) = obj
        .get_mut("definition")
        .and_then(|d| d.as_object_mut())
        .and_then(|d| d.get_mut("nodes"))
        .and_then(|n| n.as_object_mut())
    else {
        return;
    };
    for node in nodes.values_mut() {
        let Some(n) = node.as_object_mut() else {
            continue;
        };
        let has_input = n.get("input").map(|x| !x.is_null()).unwrap_or(false);
        let has_deps = n
            .get("depends_on")
            .and_then(|d| d.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        let has_fanout = n.get("fanout").map(|x| !x.is_null()).unwrap_or(false);
        if !has_input && !has_deps && !has_fanout {
            n.insert("input".into(), json!({ "from": "run_input" }));
        }
    }
}

fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Collect EVERY structural problem in one pass so a caller fixes them all at
/// once. Shape only — `validate_def` still runs the semantic rules (cycles,
/// JSON-output contracts, dependency consumption) after a successful parse.
fn collect_def_problems(v: &Value) -> Vec<String> {
    let mut p = Vec::new();
    let Some(def) = v.get("definition") else {
        p.push(
            "missing `definition` — wrap the whole DAG (nodes, output) in a top-level \
             `definition` object"
                .to_string(),
        );
        return p;
    };
    let Some(def) = def.as_object() else {
        p.push(format!(
            "`definition` must be an object, not {}",
            json_type(def)
        ));
        return p;
    };
    if let Some(ver) = def.get("version") {
        if ver.as_u64() != Some(SUPPORTED_DEF_VERSION as u64) {
            p.push(format!(
                "`definition.version` must be {SUPPORTED_DEF_VERSION} (or omit it to default)"
            ));
        }
    }
    for k in def.keys() {
        if !ALLOWED_DEF_KEYS.contains(&k.as_str()) {
            let hint = if ALLOWED_NODE_KEYS.contains(&k.as_str()) {
                format!(" — `{k}` is a NODE-level field; move it inside a node under `nodes`")
            } else {
                String::new()
            };
            p.push(format!("unknown field `{k}` at the definition level{hint}"));
        }
    }
    match def.get("nodes") {
        None => p.push(
            "missing `nodes` — an OBJECT keyed by node id: {\"<id>\": {agent, input, ...}}"
                .to_string(),
        ),
        Some(Value::Object(nodes)) if nodes.is_empty() => {
            p.push("`nodes` is empty — add at least one node".to_string())
        }
        Some(Value::Object(nodes)) => {
            for (id, node) in nodes {
                collect_node_problems(id, node, &mut p);
            }
        }
        Some(other) => p.push(format!(
            "`nodes` must be an OBJECT keyed by node id, not {}",
            json_type(other)
        )),
    }
    if def.get("output").is_none() {
        p.push(
            "missing `output` — {\"from\":\"node:<id>\"}: which node's result the run returns"
                .to_string(),
        );
    }
    p
}

fn collect_node_problems(id: &str, node: &Value, p: &mut Vec<String>) {
    let Some(n) = node.as_object() else {
        p.push(format!(
            "node `{id}` must be an object {{agent, input, depends_on?, fanout?}}, not {}",
            json_type(node)
        ));
        return;
    };
    for k in n.keys() {
        if !ALLOWED_NODE_KEYS.contains(&k.as_str()) {
            let hint = if ALLOWED_AGENT_KEYS.contains(&k.as_str()) {
                format!(" — `{k}` goes inside `agent`")
            } else {
                String::new()
            };
            p.push(format!("node `{id}`: unknown field `{k}`{hint}"));
        }
    }
    match n.get("agent") {
        None => p.push(format!(
            "node `{id}`: missing `agent` — {{\"model\":\"<id from router::models::list>\"}}"
        )),
        Some(Value::Object(agent)) => {
            for k in agent.keys() {
                if !ALLOWED_AGENT_KEYS.contains(&k.as_str()) {
                    let hint = if ALLOWED_NODE_KEYS.contains(&k.as_str()) {
                        format!(" — `{k}` is a NODE-level field, not under `agent`")
                    } else {
                        String::new()
                    };
                    p.push(format!("node `{id}`.agent: unknown field `{k}`{hint}"));
                }
            }
            let has_model = agent
                .get("model")
                .and_then(|m| m.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_model {
                p.push(format!(
                    "node `{id}`.agent: missing `model` — an id from router::models::list"
                ));
            }
        }
        Some(other) => p.push(format!(
            "node `{id}`.agent must be an object, not {}",
            json_type(other)
        )),
    }
    let has_input = n.get("input").map(|x| !x.is_null()).unwrap_or(false);
    if !has_input {
        p.push(format!(
            "node `{id}`: missing `input` — where it reads from, e.g. {{\"from\":\"node:<dep>\"}} \
             (join) or {{\"from\":\"fanout_item\"}} (fanout child)"
        ));
    }
}

fn format_problems(problems: &[String]) -> String {
    let n = problems.len();
    let mut s = format!(
        "workflow::start: the `definition` has {n} problem{}:",
        if n == 1 { "" } else { "s" }
    );
    for pr in problems {
        s.push_str("\n  - ");
        s.push_str(pr);
    }
    s.push('\n');
    s.push_str(SHAPE_HINT);
    s
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StartResponse {
    pub run_id: String,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a `WorkflowDef` for structural correctness.
///
/// Rules:
/// 0. `version` must be supported; node count is bounded; node ids must not
///    contain the reserved separators `#`/`.`; `output.from` must reference an
///    existing node.
/// 1. Every node's `agent.model` must be non-empty.
/// 2. Any node referenced by another node's `fanout.over` (strip `node:`,
///    take the part before the first `.`) OR by a node's `input.from`
///    `"node:<id>"` source (single or array) MUST have `agent.output` be a JSON
///    object with `"type" == "json"`.
/// 3. The `depends_on` graph must be acyclic (and reference existing nodes).
/// 4. Every entry of an `input.from` ARRAY (the join form) must be a
///    `"node:<id>"` reference.
/// 5. Every `depends_on` entry must be CONSUMED by the node's `input.from` or
///    `fanout.over` — a declared-but-unread dependency is silently dropped while
///    the run still reports success, so it is rejected up front.
pub fn validate_def(def: &WorkflowDef) -> Result<(), WorkflowError> {
    // Rule 0a: supported schema version.
    if def.version != SUPPORTED_DEF_VERSION {
        return Err(WorkflowError::InvalidDef(format!(
            "unsupported definition version {} (expected {})",
            def.version, SUPPORTED_DEF_VERSION
        )));
    }

    // Rule 0b: bounded node count (untrusted input).
    if def.nodes.len() > MAX_NODES {
        return Err(WorkflowError::InvalidDef(format!(
            "definition has {} nodes (max {})",
            def.nodes.len(),
            MAX_NODES
        )));
    }

    // Rule 0c: node ids must not contain the reserved separators '#' (fanout
    // item index) or '.' (over-path), which would mis-parse at dispatch time.
    for node_id in def.nodes.keys() {
        if node_id.contains('#') || node_id.contains('.') {
            return Err(WorkflowError::InvalidDef(format!(
                "node id '{}' must not contain '#' or '.'",
                node_id
            )));
        }
    }

    // Rule 0d: the output node must reference an existing node, otherwise the
    // run can never reach Completed and hangs in AwaitingNodes forever.
    let out_id = def
        .output
        .from
        .strip_prefix("node:")
        .unwrap_or(&def.output.from);
    if !def.nodes.contains_key(out_id) {
        return Err(WorkflowError::InvalidDef(format!(
            "output.from references unknown node '{}'",
            out_id
        )));
    }

    // Rule 1: all models non-empty.
    for (node_id, node) in &def.nodes {
        if node.agent.model.trim().is_empty() {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{}': agent.model must not be empty",
                node_id
            )));
        }
    }

    // Rule 1b: agent.functions, if present, must be a shape the worker can coerce
    // into a harness FunctionPolicy — an allow-list array of strings, a single
    // string, or a FunctionPolicy object. A bare array is the common form and is
    // normalized to `{ "allow": [...] }` at dispatch (see tick::normalize_functions).
    // Rejecting other shapes here turns a poison-pill (every tick dead-letters on
    // `harness::send`) into an up-front error the caller sees from workflow::start.
    for (node_id, node) in &def.nodes {
        match &node.agent.functions {
            None | Some(Value::Null) | Some(Value::Object(_)) | Some(Value::String(_)) => {}
            Some(Value::Array(items)) => {
                if let Some(bad) = items.iter().find(|v| !v.is_string()) {
                    return Err(WorkflowError::InvalidDef(format!(
                        "node '{}': agent.functions must be an array of function-id strings; \
                         found a non-string element ({})",
                        node_id, bad
                    )));
                }
            }
            Some(other) => {
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{}': agent.functions must be an allow-list array (e.g. \
                     [\"web::fetch\"]), a string, or a FunctionPolicy object; got {}",
                    node_id, other
                )));
            }
        }
    }

    // Rule 1c: a JSON output contract whose `schema` is an object must give that
    // schema a non-empty `type`. An empty `{}` (or any typeless object) is rejected
    // by the provider at runtime with "input_schema.type: Field required", which
    // fails the node mid-run; catch it up front with an actionable message.
    for (node_id, node) in &def.nodes {
        if let Some(output) = &node.agent.output {
            let is_json = output.get("type").and_then(|t| t.as_str()) == Some("json");
            if is_json {
                if let Some(schema) = output.get("schema") {
                    if schema.is_object() {
                        let has_type = schema
                            .get("type")
                            .and_then(|t| t.as_str())
                            .map(|t| !t.trim().is_empty())
                            .unwrap_or(false);
                        if !has_type {
                            return Err(WorkflowError::InvalidDef(format!(
                                "node '{}': agent.output is type \"json\" but its schema has no \
                                 top-level \"type\" — an empty/typeless schema is rejected by the \
                                 provider (\"input_schema.type: Field required\"). Use a real JSON \
                                 Schema, e.g. {{\"type\":\"object\",\"properties\":{{...}}}}.",
                                node_id
                            )));
                        }
                    }
                }
            }
        }
    }

    // Collect nodes that must have JSON output.
    let mut must_have_json_output: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for node in def.nodes.values() {
        // fanout.over: "node:<dep_id>.<path...>" → extract dep_id
        if let Some(fanout) = &node.fanout {
            if let Some(rest) = fanout.over.strip_prefix("node:") {
                let dep_id = rest.split('.').next().unwrap_or(rest);
                must_have_json_output.insert(dep_id.to_string());
            }
        }

        // input.from: each "node:<dep_id>" source (One or Many) → extract dep_id
        for src in node.input.from.sources() {
            if let Some(rest) = src.strip_prefix("node:") {
                let dep_id = rest.split('.').next().unwrap_or(rest);
                must_have_json_output.insert(dep_id.to_string());
            }
        }
    }

    // Rule 2: those deps must have output.type == "json".
    for dep_id in &must_have_json_output {
        let dep_node = def.nodes.get(dep_id.as_str()).ok_or_else(|| {
            WorkflowError::InvalidDef(format!("node '{}' referenced but not defined", dep_id))
        })?;

        let has_json_output = dep_node
            .agent
            .output
            .as_ref()
            .and_then(|o| o.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "json")
            .unwrap_or(false);

        if !has_json_output {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{}' is consumed by a fanout or node input but does not declare \
                 agent.output.type == \"json\"",
                dep_id
            )));
        }
    }

    // Rule 2b: when a fanout's `over` path can be STATICALLY PROVEN to land on a
    // non-array field in the upstream's declared output schema, reject at start.
    // Without this the run burns every upstream node, then dies at expansion time
    // in `dag::resolve_over_path` ("value at path is string (not an array)") —
    // minutes and real tokens into a paid run. We only fail when we can PROVE it
    // (a constant `node:` path landing on an explicitly-typed non-array leaf);
    // dynamic shapes ($ref / anyOf / additionalProperties / untyped / undeclared
    // segment) fall through to the runtime guard, so this never rejects a
    // legitimate def.
    for (node_id, node) in &def.nodes {
        if let Some(fanout) = &node.fanout {
            if let Some(leaf_type) = fanout_over_proven_nonarray(def, &fanout.over) {
                let dep = fanout
                    .over
                    .strip_prefix("node:")
                    .and_then(|r| r.split('.').next())
                    .unwrap_or(&fanout.over);
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{}': fanout.over = \"{}\" resolves to a {} in node '{}'s declared \
                     output schema, but a fanout iterates an ARRAY. Point `over` at a field that \
                     node '{}'s agent.output schema declares as {{\"type\":\"array\"}}. (To run \
                     several agents over ONE value — e.g. several critics over one draft — fan out \
                     over a list of lenses and pass the value as a separate input; don't fan out \
                     over the value itself.)",
                    node_id, fanout.over, leaf_type, dep, dep
                )));
            }
        }
    }

    // Rule 4: a node's `input.from` array (the `Many` join form) may contain only
    // `"node:<id>"` references — a join gathers dependency outputs, not run_input
    // or fanout_item.
    for (node_id, node) in &def.nodes {
        if let crate::types::InputFrom::Many(sources) = &node.input.from {
            for src in sources {
                if !src.starts_with("node:") {
                    return Err(WorkflowError::InvalidDef(format!(
                        "node '{}': input.from array entries must be node references like \
                         \"node:<id>\"; got \"{}\"",
                        node_id, src
                    )));
                }
            }
        }
    }

    // Rule 4b: an `input.from` `"node:<id>"` source must be a BARE node id — no
    // dotted path. Only `fanout.over` walks a dotted path; `input.from` gathers a
    // node's WHOLE result. A dotted form (e.g. "node:plan.result.docs") passes the
    // de-dotted dep checks in Rules 2/5 yet `dag::gather_one` keys the entire
    // remainder (it never splits on '.'), so at runtime it silently resolves to
    // Value::Null — the node runs on empty input while the run still reports success.
    // Reject it at start instead of producing a degenerate result.
    for (node_id, node) in &def.nodes {
        for src in node.input.from.sources() {
            if let Some(rest) = src.strip_prefix("node:") {
                if rest.contains('.') {
                    let base = rest.split('.').next().unwrap_or(rest);
                    return Err(WorkflowError::InvalidDef(format!(
                        "node '{}': input.from = \"{}\" must reference a whole node result as \
                         \"node:{}\" — a dotted path is only valid on fanout.over, and here it \
                         silently resolves to null at runtime. Drop the path suffix.",
                        node_id, src, base
                    )));
                }
            }
        }
    }

    // Rule 5: every declared dependency must actually be CONSUMED by this node's
    // `input.from` or `fanout.over`. `depends_on` is only the scheduling barrier;
    // if a dep is listed but never read, its output is silently dropped while the
    // run still reports success — the multi-input-join footgun. A join node must
    // read all its deps, e.g. `input.from: ["node:a", "node:b"]`.
    for (node_id, node) in &def.nodes {
        let mut consumed: BTreeSet<&str> = BTreeSet::new();
        for src in node.input.from.sources() {
            if let Some(rest) = src.strip_prefix("node:") {
                consumed.insert(rest.split('.').next().unwrap_or(rest));
            }
        }
        if let Some(fanout) = &node.fanout {
            if let Some(rest) = fanout.over.strip_prefix("node:") {
                consumed.insert(rest.split('.').next().unwrap_or(rest));
            }
        }
        for dep in &node.depends_on {
            if !consumed.contains(dep.as_str()) {
                let reads = match &node.input.from {
                    crate::types::InputFrom::One(s) => format!("\"{}\"", s),
                    crate::types::InputFrom::Many(v) => format!("[{}]", v.join(", ")),
                };
                let fan = node
                    .fanout
                    .as_ref()
                    .map(|f| format!(", fanout.over = \"{}\"", f.over))
                    .unwrap_or_default();
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{}' depends_on '{}' but never consumes it (input.from = {}{}). \
                     A declared dependency whose output is not read is silently dropped; read \
                     it via input.from (use an array like [\"node:{}\", …] to join several \
                     deps) or remove it from depends_on.",
                    node_id, dep, reads, fan, dep
                )));
            }
        }
    }

    // Rule 3: no cycles.
    crate::dag::validate_acyclic(def).map_err(WorkflowError::InvalidDef)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Static fanout-type checking (Rule 2b)
// ---------------------------------------------------------------------------

/// JSON Schema composition / dynamic keys that make a schema's effective type
/// impossible to pin down statically. If any is present we cannot prove the
/// shape, so the static fanout check bails and the runtime guard takes over.
fn schema_is_dynamic(schema: &Value) -> bool {
    const DYNAMIC_KEYS: [&str; 7] = [
        "additionalProperties",
        "patternProperties",
        "$ref",
        "$dynamicRef",
        "anyOf",
        "oneOf",
        "allOf",
    ];
    DYNAMIC_KEYS.iter().any(|k| schema.get(*k).is_some())
}

/// If `schema`'s declared `type` PROVES the value is not (and cannot be) an
/// array, return that type name; otherwise None (it is an array, a union that
/// includes "array", untyped, or a dynamic schema we won't second-guess).
fn proven_nonarray_type(schema: &Value) -> Option<String> {
    if schema_is_dynamic(schema) {
        return None;
    }
    match schema.get("type") {
        Some(Value::String(t)) if t != "array" => Some(t.clone()),
        Some(Value::Array(types)) => {
            let names: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
            if names.is_empty() || names.contains(&"array") {
                None
            } else {
                Some(names.join("|"))
            }
        }
        _ => None,
    }
}

/// Walk a `fanout.over` path (`"node:<id>.<seg>.<seg>"`) into the upstream node's
/// DECLARED output schema and, if the leaf is provably non-array, return its type
/// name. Mirrors `dag::resolve_over_path`'s path protocol (skip a leading
/// `"result"` segment) but walks the schema instead of a runtime value. Returns
/// None whenever the type can't be proven — missing node/schema, a non-JSON
/// output, an undeclared segment, a dynamic schema, or an array leaf — so the
/// run proceeds and the runtime guard decides.
fn fanout_over_proven_nonarray(def: &WorkflowDef, over: &str) -> Option<String> {
    let path = over.strip_prefix("node:")?;
    let mut it = path.splitn(2, '.');
    let node_id = it.next()?;
    let rest = it.next().unwrap_or("");

    let dep = def.nodes.get(node_id)?;
    let output = dep.agent.output.as_ref()?;
    if output.get("type").and_then(Value::as_str) != Some("json") {
        return None;
    }
    let schema = output.get("schema")?;
    if !schema.is_object() {
        return None;
    }

    let segments: Vec<&str> = rest.split('.').filter(|s| !s.is_empty()).collect();
    let walk: &[&str] = if segments.first() == Some(&"result") {
        &segments[1..]
    } else {
        &segments[..]
    };

    let mut cur = schema;
    for seg in walk {
        if schema_is_dynamic(cur) {
            return None;
        }
        cur = cur.get("properties")?.get(*seg)?;
    }
    proven_nonarray_type(cur)
}

// ---------------------------------------------------------------------------
// Model validation (against the live router catalog)
// ---------------------------------------------------------------------------

/// Distinct node `model`s not registered on this engine, sorted. When a node pins
/// `agent.provider`, the `(provider, model)` PAIR must be registered — a model
/// served only by a *different* provider does NOT count (otherwise the node fails
/// mid-run with "no provider registered for model"). A node with no provider pin is
/// satisfied by the model id under any provider. Pure, so it's unit-testable.
pub fn unknown_models(def: &WorkflowDef, registered: &BTreeSet<(String, String)>) -> Vec<String> {
    let ids: BTreeSet<&str> = registered.iter().map(|(_, id)| id.as_str()).collect();
    let mut bad: BTreeSet<String> = BTreeSet::new();
    for node in def.nodes.values() {
        let m = node.agent.model.trim();
        if m.is_empty() {
            continue;
        }
        let pinned = node
            .agent
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        let known = match pinned {
            Some(p) => registered.contains(&(p.to_string(), m.to_string())),
            None => ids.contains(m),
        };
        if !known {
            bad.insert(match pinned {
                Some(p) => format!("{m} (provider {p})"),
                None => m.to_string(),
            });
        }
    }
    bad.into_iter().collect()
}

/// Read the set of registered `(provider, model-id)` pairs from
/// `router::models::list`. Returns None if the catalog can't be read (router down
/// / unexpected shape / empty) — the caller then SKIPS model validation rather
/// than blocking a legitimate run; the node-level failure surfaced via
/// `result_error` remains the backstop.
async fn fetch_registered_models(deps: &Deps) -> Option<BTreeSet<(String, String)>> {
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    let resp = deps
        .iii
        .trigger(iii_sdk::protocol::TriggerRequest {
            function_id: "router::models::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .ok()?;

    let set: BTreeSet<(String, String)> = resp
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?.to_string();
            let provider = m.get("provider").and_then(|v| v.as_str())?.to_string();
            Some((provider, id))
        })
        .collect();

    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

/// Reject a definition that names a model with no provider registered on this
/// engine, naming the offenders and the available ids. Fail-fast at start beats a
/// per-node "no provider registered for model …" at runtime. Lenient when the
/// catalog is unreadable (router not up): skip rather than block.
async fn validate_models(deps: &Deps, def: &WorkflowDef) -> Result<(), WorkflowError> {
    let Some(registered) = fetch_registered_models(deps).await else {
        tracing::warn!("router::models::list unavailable; skipping model validation");
        return Ok(());
    };

    let unknown = unknown_models(def, &registered);
    if !unknown.is_empty() {
        let available: BTreeSet<&str> = registered.iter().map(|(_, id)| id.as_str()).collect();
        let available: Vec<&str> = available.into_iter().collect();
        return Err(WorkflowError::InvalidDef(format!(
            "unknown model(s) {:?}: not registered on this engine. Available: {}. \
             Discover with router::models::list.",
            unknown,
            available.join(", ")
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Enqueue helper
// ---------------------------------------------------------------------------

pub async fn enqueue_tick(
    iii: &iii_sdk::IIIClient,
    run_id: &str,
    step: u64,
) -> Result<(), WorkflowError> {
    iii.trigger(iii_sdk::protocol::TriggerRequest {
        function_id: "workflow::tick".into(),
        payload: json!({"run_id": run_id, "step": step}),
        action: Some(iii_sdk::TriggerAction::Enqueue {
            queue: "default".into(),
        }),
        timeout_ms: None,
    })
    .await
    .map_err(|e| WorkflowError::Trigger(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Namespace an idempotency key by its caller. The key is stored in a flat global
/// keyspace (`SCOPE_IDEM`), so an un-scoped key lets two different callers using
/// the same string (e.g. "daily-report") collide: the second caller is handed
/// back the first's `run_id`, leaking it — and, via `workflow::status` /
/// `workflow::node-result`, the run's results. `caller_session_id` is hook-stamped
/// from the caller's real session (never trusted from the agent), and session ids
/// contain no `|`, so the prefix unambiguously isolates callers. Non-agent (trusted)
/// callers share the `_anon` namespace.
fn scoped_idem_key(key: &str, caller_session_id: Option<&str>) -> String {
    format!("{}|{}", caller_session_id.unwrap_or("_anon"), key)
}

/// Count the workflow runs in a caller's ancestry, to bound sub-workflow nesting.
/// Every workflow node session is reverse-indexed to its run (state::put_session_index
/// in fire_node), and every run records the session that started it
/// (caller_session_id), so we walk the chain — node session → its run → that run's
/// caller → … — counting how deep a NEW run started by this caller would sit. A
/// non-node caller (a real chat/console session) misses the reverse index and ends
/// the chain at depth 0, so a normal top-level start costs a single lookup. Bounded
/// to MAX_WORKFLOW_DEPTH+1 iterations so a malformed/cyclic chain can't loop forever.
async fn caller_workflow_depth(
    deps: &Deps,
    caller_session_id: Option<&str>,
) -> Result<usize, WorkflowError> {
    let mut depth = 0usize;
    let mut cur = caller_session_id.map(str::to_string);
    while let Some(sid) = cur {
        match state::run_id_for_session(&deps.iii, &sid).await? {
            Some(parent_run) => {
                depth += 1;
                if depth > MAX_WORKFLOW_DEPTH {
                    break;
                }
                cur = state::get_run(&deps.iii, &parent_run)
                    .await?
                    .and_then(|r| r.caller_session_id);
            }
            // Caller is not a workflow node session — the ancestry chain ends.
            None => break,
        }
    }
    Ok(depth)
}

/// Start a run: validate, resolve the caller session, dedupe on the idempotency
/// key, persist the Running record, and enqueue the first tick. Fire-and-forget —
/// the caller gets the `run_id` back immediately and receives the outcome via
/// `reply_to` / `notify` (or by polling `workflow::status`); the harness turn is
/// never blocked.
pub async fn handle(deps: &Deps, req: StartRequest) -> Result<StartResponse, WorkflowError> {
    validate_def(&req.definition)?;

    // Resolve the caller/orchestrator session ONCE: the hook-stamped caller, then a
    // reply_to target; `None` for a non-agent caller. Used both to nest node sessions
    // in the console AND to scope the idempotency key (keeping its GET and PUT out of
    // the shared `_anon` bucket, so two callers reusing one key can't collide).
    let caller_session_id = req
        .caller_session_id
        .clone()
        .or_else(|| req.reply_to.as_ref().and_then(|r| r.session_id.clone()));

    // Idempotency short-circuit.
    if let Some(ref key) = req.idempotency_key {
        if key.len() > MAX_IDEM_KEY_LEN {
            return Err(WorkflowError::InvalidDef(format!(
                "idempotency_key too long: {} bytes (max {})",
                key.len(),
                MAX_IDEM_KEY_LEN
            )));
        }
        let scoped = scoped_idem_key(key, caller_session_id.as_deref());
        if let Some(existing_run_id) = state::get_idem(&deps.iii, &scoped).await? {
            return Ok(StartResponse {
                run_id: existing_run_id,
            });
        }
    }

    // Fail fast on a model with no provider on this engine: otherwise the node's
    // child turn fails at runtime ("no provider registered for model …") and the
    // whole run fails. Lenient if the catalog can't be read.
    validate_models(deps, &req.definition).await?;

    let run_id = new_run_id();
    let _guard = deps.locks.guard(&run_id).await;

    state::put_def(&deps.iii, &run_id, &req.definition).await?;

    // Bound sub-workflow nesting: a node that opted into `workflow::start` could
    // otherwise recurse (sub-workflow → node → sub-workflow → …) without limit.
    let depth = caller_workflow_depth(deps, caller_session_id.as_deref()).await?;
    if depth > MAX_WORKFLOW_DEPTH {
        return Err(WorkflowError::InvalidDef(format!(
            "sub-workflow nesting depth {depth} exceeds the cap of {MAX_WORKFLOW_DEPTH}"
        )));
    }

    let now = deps.now_ms();
    let mut record = WorkflowRunRecord {
        run_id: run_id.clone(),
        step: 0,
        status: RunStatus::Running,
        abort: false,
        def_ref: run_id.clone(),
        input: req.input,
        nodes: BTreeMap::new(),
        fanout_src: BTreeMap::new(),
        result: None,
        result_error: None,
        notify: req.notify,
        reply_to: req.reply_to,
        caller_session_id,
        created_at: now,
        updated_at: now,
    };

    state::put_run(&deps.iii, &record).await?;

    crate::telemetry::record_run_started();

    if let Some(ref key) = req.idempotency_key {
        let scoped = scoped_idem_key(key, record.caller_session_id.as_deref());
        state::put_idem(&deps.iii, &scoped, &run_id).await?;
    }

    // The Running record is already persisted; if the first tick fails to enqueue,
    // only the cron sweep would recover it (up to a sweep interval later). Mark the
    // run Failed best-effort so workflow::status / list don't surface a phantom
    // Running run in the meantime.
    if let Err(e) = enqueue_tick(&deps.iii, &run_id, 0).await {
        record.status = RunStatus::Failed;
        record.result_error = Some(format!("failed to enqueue initial tick: {e}"));
        record.updated_at = deps.now_ms();
        let _ = state::put_run(&deps.iii, &record).await;
        return Err(e);
    }

    Ok(StartResponse { run_id })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AgentSpec, FanoutSpec, InputFrom, InputSpec, NodeDef, OutputRef, WorkflowDef,
    };
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};

    fn make_node(
        model: &str,
        output: Option<Value>,
        fanout_over: Option<&str>,
        input_from: &str,
    ) -> NodeDef {
        NodeDef {
            agent: AgentSpec {
                model: model.to_string(),
                provider: None,
                system_prompt: None,
                functions: None,
                output,
            },
            input: InputSpec {
                from: input_from.into(),
                template: None,
            },
            depends_on: vec![],
            fanout: fanout_over.map(|over| FanoutSpec {
                over: over.to_string(),
            }),
        }
    }

    /// A well-formed 3-node fan-out/barrier def:
    ///   plan (output: json) → read (fanout over plan.result.docs, output: json) → summarize (barrier)
    fn well_formed_def() -> WorkflowDef {
        let mut nodes = BTreeMap::new();

        nodes.insert(
            "plan".to_string(),
            make_node(
                "claude-3-5-haiku-20241022",
                Some(json!({"type": "json", "schema": {"type": "object"}})),
                None,
                "workflow.input",
            ),
        );

        nodes.insert(
            "read".to_string(),
            make_node(
                "claude-3-5-haiku-20241022",
                Some(json!({"type": "json", "schema": {"type": "object"}})),
                Some("node:plan.result.docs"),
                "node:plan",
            ),
        );

        nodes.insert(
            "summarize".to_string(),
            make_node("claude-3-5-haiku-20241022", None, None, "node:read"),
        );

        WorkflowDef {
            version: 1,
            nodes,
            output: OutputRef {
                from: "node:summarize".into(),
            },
            default_functions: None,
        }
    }

    #[test]
    fn rejects_missing_model() {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.model = "".to_string();
        assert!(
            validate_def(&def).is_err(),
            "expected Err for empty model, got Ok"
        );
    }

    #[test]
    fn rejects_output_referencing_unknown_node() {
        // C1: an output.from pointing at a non-existent node would hang the run.
        let mut def = well_formed_def();
        def.output.from = "node:ghost".to_string();
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(
            err.contains("ghost"),
            "error should name the missing node: {err}"
        );
    }

    #[test]
    fn rejects_node_id_with_reserved_separator() {
        // C5: '#'/'.' in a node id collides with fanout-item / over-path parsing.
        let mut def = well_formed_def();
        def.nodes.insert(
            "bad#id".to_string(),
            make_node("claude-3-5-haiku-20241022", None, None, "run_input"),
        );
        assert!(
            validate_def(&def).is_err(),
            "expected Err for node id with '#'"
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut def = well_formed_def();
        def.version = 2;
        assert!(
            validate_def(&def).is_err(),
            "expected Err for unsupported version"
        );
    }

    #[test]
    fn rejects_node_feeding_fanout_without_json_output() {
        let mut def = well_formed_def();
        // Remove plan's json output so it fails the fanout rule.
        def.nodes.get_mut("plan").unwrap().agent.output = None;
        let result = validate_def(&def);
        assert!(
            result.is_err(),
            "expected Err when plan feeds fanout but has no json output, got Ok"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("plan"),
            "error should mention 'plan', got: {}",
            msg
        );
    }

    #[test]
    fn accepts_well_formed_def() {
        let def = well_formed_def();
        assert!(
            validate_def(&def).is_ok(),
            "expected Ok for well-formed def, got Err"
        );
    }

    #[test]
    fn rejects_depends_on_that_is_never_consumed() {
        // The multi-input-join footgun: summarize depends_on both read and plan,
        // but its single `from` only reads read — plan's output would be silently
        // dropped while the run still reports success.
        let mut def = well_formed_def();
        let summarize = def.nodes.get_mut("summarize").unwrap();
        summarize.depends_on = vec!["read".to_string(), "plan".to_string()];
        // input.from stays "node:read" (set by make_node) — plan is unconsumed.
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("summarize"), "error must name the node: {err}");
        assert!(
            err.contains("plan"),
            "error must name the dropped dep: {err}"
        );
        assert!(
            err.contains("never consumes"),
            "error must explain the drop: {err}"
        );
    }

    #[test]
    fn accepts_join_consuming_all_deps_via_input_array() {
        // The fix: a join reads BOTH deps via the `Many` array form.
        let mut def = well_formed_def();
        let summarize = def.nodes.get_mut("summarize").unwrap();
        summarize.depends_on = vec!["read".to_string(), "plan".to_string()];
        summarize.input.from =
            InputFrom::Many(vec!["node:read".to_string(), "node:plan".to_string()]);
        assert!(
            validate_def(&def).is_ok(),
            "a join consuming all its deps must be accepted: {:?}",
            validate_def(&def)
        );
    }

    #[test]
    fn rejects_input_array_with_non_node_entry() {
        let mut def = well_formed_def();
        let summarize = def.nodes.get_mut("summarize").unwrap();
        summarize.depends_on = vec!["read".to_string()];
        summarize.input.from =
            InputFrom::Many(vec!["node:read".to_string(), "run_input".to_string()]);
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(
            err.contains("node references"),
            "error must explain the array must be node refs: {err}"
        );
    }

    #[test]
    fn unknown_models_flags_unregistered_then_clears() {
        // well_formed_def uses model "claude-3-5-haiku-20241022" on every node, no provider pin.
        let def = well_formed_def();
        let mut registered: BTreeSet<(String, String)> = BTreeSet::new();
        registered.insert(("anthropic".to_string(), "claude-sonnet-4-6".to_string()));
        assert_eq!(
            unknown_models(&def, &registered),
            vec!["claude-3-5-haiku-20241022".to_string()],
            "the def's model is not in the catalog → flagged once (deduped)"
        );

        registered.insert((
            "anthropic".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ));
        assert!(
            unknown_models(&def, &registered).is_empty(),
            "once the model is registered (node pins no provider), nothing is flagged"
        );
    }

    #[test]
    fn unknown_models_respects_provider_pin() {
        // A node pinning a provider that doesn't serve the model is flagged even
        // though the model id IS registered under a different provider.
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.provider = Some("groq".to_string());
        let mut registered: BTreeSet<(String, String)> = BTreeSet::new();
        registered.insert((
            "anthropic".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ));
        assert!(
            unknown_models(&def, &registered)
                .contains(&"claude-3-5-haiku-20241022 (provider groq)".to_string()),
            "plan pins groq, which doesn't serve the model registered under anthropic → flagged"
        );

        registered.insert(("groq".to_string(), "claude-3-5-haiku-20241022".to_string()));
        assert!(
            unknown_models(&def, &registered).is_empty(),
            "once groq serves the model, the pinned node is satisfied"
        );
    }

    #[test]
    fn start_request_parses_notify_callback() {
        // Regression: a caller must be able to register a completion callback at
        // start time (push) instead of polling workflow::status.
        let req: StartRequest = serde_json::from_value(json!({
            "definition": well_formed_def(),
            "notify": { "function_id": "myworker::wf-done" }
        }))
        .expect("StartRequest with notify");
        let notify = req.notify.expect("notify present");
        assert_eq!(notify.function_id, "myworker::wf-done");
        assert!(
            notify.queue.is_none(),
            "queue defaults to None (→ \"default\")"
        );
    }

    #[test]
    fn start_request_parses_reply_to() {
        // A caller opts into message-delivery with `reply_to`; session/model are
        // auto-stamped later by the pre_trigger hook, not supplied here.
        let req: StartRequest = serde_json::from_value(json!({
            "definition": well_formed_def(),
            "reply_to": { "template": "Pick the winner:" }
        }))
        .expect("StartRequest with reply_to");
        let reply = req.reply_to.expect("reply_to present");
        assert_eq!(reply.template.as_deref(), Some("Pick the winner:"));
        assert!(reply.session_id.is_none());
    }

    #[test]
    fn start_request_reply_to_is_optional() {
        let req: StartRequest = serde_json::from_value(json!({
            "definition": well_formed_def()
        }))
        .expect("StartRequest without reply_to");
        assert!(req.reply_to.is_none());
        assert!(req.caller_session_id.is_none());
    }

    #[test]
    fn start_request_parses_caller_session_id() {
        // The pre_trigger hook stamps `caller_session_id` into the args (from the
        // caller's turn) so node sessions can nest under the orchestrator.
        let req: StartRequest = serde_json::from_value(json!({
            "definition": well_formed_def(),
            "caller_session_id": "console-abc"
        }))
        .expect("StartRequest with caller_session_id");
        assert_eq!(req.caller_session_id.as_deref(), Some("console-abc"));
    }

    #[test]
    fn start_request_notify_is_optional() {
        let req: StartRequest = serde_json::from_value(json!({
            "definition": well_formed_def()
        }))
        .expect("StartRequest without notify");
        assert!(req.notify.is_none());
    }

    #[test]
    fn await_flag_is_dropped_and_ignored() {
        // The blocking `await` mechanism was removed (the harness must never be
        // parked). A stale caller still passing `await: true` must not error — the
        // key is simply ignored and the run is the normal fire-and-forget start.
        let schema = serde_json::to_value(schemars::schema_for!(StartRequest)).unwrap();
        assert!(
            schema["properties"].get("await").is_none(),
            "`await` must not appear in the request schema: {schema:#}"
        );

        let req: StartRequest = serde_json::from_value(json!({
            "definition": well_formed_def(),
            "await": true
        }))
        .expect("a stray `await` key must be ignored, not rejected");
        // Nothing on the request carries it through — it's gone.
        assert!(req.reply_to.is_none() && req.notify.is_none());
    }

    #[test]
    fn definition_accepts_workflow_alias() {
        // A model that guessed the wrapper key `workflow` (the worker's own name)
        // instead of `definition` still parses — this is the exact key the failing
        // local-model session used.
        let req: StartRequest = serde_json::from_value(json!({
            "workflow": {
                "version": 1,
                "nodes": { "a": { "agent": { "model": "m" }, "input": { "from": "run_input" } } },
                "output": { "from": "node:a" }
            }
        }))
        .expect("`workflow` should alias to `definition`");
        assert!(req.definition.nodes.contains_key("a"));
    }

    #[test]
    fn collects_all_structural_problems_at_once() {
        // The exact failure mode from the local-model session: every node missing
        // `input`, no `output`. One error must list them ALL (no whack-a-mole).
        let err = serde_json::from_value::<StartRequest>(json!({
            "definition": {
                "nodes": {
                    "gen": { "agent": { "model": "m" }, "fanout": { "over": "node:x.items" } },
                    "crit": { "agent": { "model": "m" }, "depends_on": ["gen"] }
                }
            }
        }))
        .unwrap_err();
        let msg = err.to_string();
        // Both non-source nodes flagged for missing input, plus missing output.
        assert!(msg.contains("node `gen`: missing `input`"), "msg: {msg}");
        assert!(msg.contains("node `crit`: missing `input`"), "msg: {msg}");
        assert!(msg.contains("missing `output`"), "msg: {msg}");
        assert!(msg.contains("3 problems"), "msg: {msg}");
    }

    #[test]
    fn defaults_version_and_source_node_input() {
        // A pure source node may omit `input` (defaults to run_input) and the whole
        // def may omit `version` (defaults to 1).
        let req: StartRequest = serde_json::from_value(json!({
            "definition": {
                "nodes": { "only": { "agent": { "model": "m" } } },
                "output": { "from": "node:only" }
            }
        }))
        .expect("source-node input + version should default");
        assert_eq!(req.definition.version, 1);
        let node = req.definition.nodes.get("only").unwrap();
        assert!(matches!(&node.input.from, crate::types::InputFrom::One(s) if s == "run_input"));
    }

    #[test]
    fn flags_misplaced_fields_with_hints() {
        // `input` at the def level and `fanout` under `agent` — common placement
        // mistakes that deny_unknown_fields would otherwise report one at a time.
        let err = serde_json::from_value::<StartRequest>(json!({
            "definition": {
                "input": { "from": "run_input" },
                "nodes": { "n": { "agent": { "model": "m", "fanout": { "over": "x" } }, "input": { "from": "run_input" } } },
                "output": { "from": "node:n" }
            }
        }))
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field `input` at the definition level"),
            "msg: {msg}"
        );
        assert!(
            msg.contains("node `n`.agent: unknown field `fanout`"),
            "msg: {msg}"
        );
        assert!(msg.contains("NODE-level"), "msg: {msg}");
    }

    #[test]
    fn accepts_functions_allow_list_array() {
        // The natural shorthand an agent writes; normalized to {allow:[...]} at dispatch.
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.functions = Some(json!(["web::fetch"]));
        assert!(
            validate_def(&def).is_ok(),
            "allow-list array should be accepted"
        );
    }

    #[test]
    fn rejects_functions_with_non_string_element() {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.functions = Some(json!(["web::fetch", 7]));
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("plan"), "error should name the node: {err}");
        assert!(
            err.contains("non-string"),
            "error should explain the cause: {err}"
        );
    }

    #[test]
    fn rejects_functions_of_wrong_type() {
        // A number is neither an allow-list, a string, nor a FunctionPolicy object.
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.functions = Some(json!(42));
        assert!(
            validate_def(&def).is_err(),
            "a number is not a valid functions shape"
        );
    }

    #[test]
    fn rejects_json_output_with_empty_schema() {
        // The live foot-gun: {type:json, schema:{}} fails at the provider with
        // "input_schema.type: Field required". Catch it at start.
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.output =
            Some(json!({"type": "json", "schema": {}}));
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("plan"), "names the node: {err}");
        assert!(err.contains("type"), "explains the missing type: {err}");
    }

    #[test]
    fn rejects_json_output_with_typeless_schema() {
        let mut def = well_formed_def();
        // properties but no top-level "type" → still invalid as a tool input_schema.
        def.nodes.get_mut("plan").unwrap().agent.output =
            Some(json!({"type": "json", "schema": {"properties": {"x": {"type": "string"}}}}));
        assert!(
            validate_def(&def).is_err(),
            "a schema without a top-level type must be rejected"
        );
    }

    #[test]
    fn accepts_json_output_with_typed_schema() {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.output = Some(
            json!({"type": "json", "schema": {"type": "object", "properties": {"x": {"type": "string"}}}}),
        );
        assert!(validate_def(&def).is_ok(), "a typed object schema is valid");
    }

    #[test]
    fn rejects_fanout_over_non_array_leaf() {
        // The live foot-gun (run r_fe25cffc): a node fans out over a field its
        // upstream declares as a string. A fanout needs an array; reject at start
        // instead of after every upstream node has already run.
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.output = Some(json!({
            "type": "json",
            "schema": {"type": "object", "properties": {"docs": {"type": "string"}}}
        }));
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("read"), "names the fanout node: {err}");
        assert!(err.contains("string"), "names the proven leaf type: {err}");
        assert!(
            err.contains("array"),
            "explains a fanout needs an array: {err}"
        );
    }

    #[test]
    fn accepts_fanout_over_array_leaf() {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.output = Some(json!({
            "type": "json",
            "schema": {"type": "object", "properties": {"docs": {"type": "array"}}}
        }));
        assert!(
            validate_def(&def).is_ok(),
            "fanout over an array-typed field is valid"
        );
    }

    #[test]
    fn fanout_over_unprovable_leaf_defers_to_runtime() {
        // A union that includes "array" (or any dynamic schema) can't be proven
        // non-array statically, so start must NOT reject it — the runtime guard
        // decides once the real value exists. The default well_formed_def (plan's
        // schema is a typeless `{"type":"object"}` with no `properties`) already
        // exercises the missing-segment bail; this covers the dynamic-leaf bail.
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.output = Some(json!({
            "type": "json",
            "schema": {"type": "object", "properties": {
                "docs": {"anyOf": [{"type": "array"}, {"type": "null"}]}
            }}
        }));
        assert!(
            validate_def(&def).is_ok(),
            "an unprovable (dynamic) leaf must defer to the runtime guard"
        );
    }

    #[test]
    fn rejects_cyclic_def() {
        // b -> c, c -> b
        let d: WorkflowDef = serde_json::from_value(serde_json::json!({
            "version":1, "output":{"from":"node:b"}, "nodes":{
              "b":{"depends_on":["c"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:c","template":"t"}},
              "c":{"depends_on":["b"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:b","template":"t"}}
            }})).unwrap();
        assert!(validate_def(&d).is_err());
    }
}
