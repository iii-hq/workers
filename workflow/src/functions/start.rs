use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    error::WorkflowError,
    ids::new_run_id,
    state,
    types::{
        AgentSpec, FanoutSpec, InputFrom, InputSpec, LoopSpec, NodeDef, NotifySpec, OnError,
        OutputFrom, OutputRef, RunStatus, WorkflowDef, WorkflowRunRecord,
    },
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
const MIN_NODE_TIMEOUT_MS: u64 = 1000;
const MAX_NODE_RETRIES: u32 = 10;
const MAX_LABEL_CHARS: usize = 80;
// One digit in `#it<k>`, so every listing sorts passes in order.
const MAX_LOOP_REPEATS: u32 = 9;

// ---------------------------------------------------------------------------
// Request / Response
// ---------------------------------------------------------------------------

/// A validated `workflow::start` request, lowered to the stored definition.
/// Its `Deserialize` runs the shape pass (every problem at once), the typed
/// parse of [`RequestWire`], then [`lower`]; its published schema is the wire's.
#[derive(Debug, Clone)]
pub struct StartRequest {
    pub definition: WorkflowDef,
    pub input: Value,
    pub idempotency_key: Option<String>,
    pub notify: Option<NotifySpec>,
    pub caller_session_id: Option<String>,
    pub caller_fs_root: Option<String>,
}

// No `deny_unknown_fields` on the wire structs: a closed published schema lets the
// harness drop a misplaced key and start the run; the shape pass refuses unknown keys.

/// Run a DAG of sub-agent sessions, durable and crash-resumable. Example:
/// `{"nodes":{"a":{"task":"<what a does>"},"b":{"task":"<what b does>","reads":"a"}},"input":<any JSON>}`.
#[derive(Deserialize, JsonSchema)]
struct RequestWire {
    /// One sub-agent session per node, keyed by node id; an id must not contain
    /// `#`, `.` or `/`.
    nodes: BTreeMap<String, Node>,
    /// Any JSON value. Every node without `reads` or `fanout` receives it.
    #[serde(default)]
    input: Value,
    /// What the run returns: a node id returns that node's result as is, an array
    /// of ids an object keyed by id. Omit it to return every node no other node
    /// reads (one such node: its result as is).
    output: Option<NodeRefs>,
    /// At most this many node sessions live at once (running, or reserved for a
    /// send); further ready nodes wait for a slot. At least 1; unset means no cap.
    max_concurrency: Option<u32>,
    /// Dispatch policy inherited by every node that omits `functions` (same
    /// shape); when unset those nodes get everything except `workflow::*`,
    /// `configuration::*`, `approval::*`, `harness::hook::*` and `harness::send`.
    default_functions: Option<Value>,
    /// Dedupe key; a repeated key returns the original `run_id` instead of launching
    /// a duplicate run.
    idempotency_key: Option<String>,
    /// Completion callback called once when the run reaches a terminal state,
    /// instead of polling `workflow::status`.
    notify: Option<NotifySpec>,
    /// Orchestrator session, stamped by the `workflow::stamp-reply` hook from the
    /// caller's turn; never trusted from the agent.
    // Recorded on the run so node sessions can nest under it in the console.
    caller_session_id: Option<String>,
    /// Caller's folder. On a call from an agent turn the `workflow::stamp-reply`
    /// hook replaces it with the turn's `metadata.fs_scope.root`; the harness
    /// refuses `workflow::start` as a binding target and `fp` as a pipe step,
    /// since both skip that hook. Every node session runs in it. Omit it to use
    /// the harness default folder.
    caller_fs_root: Option<String>,
}

/// One sub-agent session. Its opening message is `task`, followed by its input
/// as JSON, fenced as untrusted data.
#[derive(Deserialize, JsonSchema)]
struct Node {
    /// What this node does: the text of its opening message, where the
    /// instructions go.
    task: String,
    /// The node(s) whose result this node receives; it runs after them. One id
    /// delivers that result as is (a text node arrives as a JSON string, a
    /// fanout node as the array of its copies' results); an array of ids
    /// delivers an object keyed by id. Omit it to receive the run `input`. On a
    /// `fanout` node the value arrives under `input`.
    reads: Option<NodeRefs>,
    /// One copy of this node runs per element of an array: `"<id>.<path>"` names
    /// an array in that node's JSON result (that node needs `schema`), or write
    /// the array itself. Without `reads` a copy receives its element; with
    /// `reads` it receives `{"input": <what reads delivers>, "fanout_item": <its
    /// element>}`. The node's result is the array of its copies' results, in
    /// element order. Not allowed on a node of a loop.
    fanout: Option<Fanout>,
    /// JSON Schema, with a top-level `type`, of the JSON value this node
    /// delivers; declare every field read downstream. Omit it for a text node.
    schema: Option<Map<String, Value>>,
    /// Omit to run this node on your own model. Otherwise a model id registered
    /// on this engine (discover them with `router::models::list`); an
    /// unregistered id is refused at start.
    model: Option<String>,
    /// Provider override; defaults to the routed provider for the model. Only
    /// with `model` or `profile`.
    provider: Option<String>,
    /// Agent profile id (`directory::agents::*`: lowercase letters, digits, `-`
    /// and `_`, at most 64 characters), sent as the harness `options.agent`. The
    /// profile supplies the system prompt, so `system_prompt` must be omitted,
    /// and a model it pins (also through `extends`) wins over `model`. The
    /// profile grants no permissions: `functions` still decides them.
    profile: Option<String>,
    /// System prompt appended to the child's built-in identity prompt.
    system_prompt: Option<String>,
    /// Child dispatch policy: an allow-list array, a single string, or
    /// `{ "allow": [...], "deny": [...] }`; omit to inherit `default_functions`.
    functions: Option<Value>,
    /// Absolute folder the node's session runs in, in place of the caller's
    /// folder. It must lie inside the caller's folder (symlinks resolved), which
    /// is checked once, at start; a run without a caller folder cannot set it.
    /// Fanout copies and loop passes share it.
    filesystem_root: Option<String>,
    /// Bounded do-while loop carried by this node (the loop node): this node and
    /// its `body` run again, as new sessions, while `while` holds on this node's
    /// result. Outside the loop only this node can be read, and readers get its
    /// last pass. Put the loop node in `output`, or read it from a later node.
    /// After the run, a result whose `while.field` still equals `equals` means the
    /// loop gave up: the repeats ran out, or a node of the last pass failed (see
    /// `failed_nodes`). Any other value means the condition stopped holding. The
    /// pass count is the number of `<id>#it<k>` uids in `nodes`, plus one. Say in
    /// the `task` of each node of the loop how to use `loop_previous` (for
    /// example: fix every finding in it), and in a body node whose result is the
    /// work itself, to build on `own_previous`, its own result of the previous
    /// pass.
    #[serde(rename = "loop")]
    loop_spec: Option<LoopSpec>,
    /// Display name, at most 80 characters: the node session's title (a fanout
    /// copy gets ` #<i>`) and the node's entry in `workflow::status` details.
    label: Option<String>,
    /// Group name, at most 80 characters; `workflow::status` counts the nodes of
    /// each phase (total, done, failed, running).
    phase: Option<String>,
    /// Milliseconds an attempt may run, wall clock from its send (harness queue
    /// wait and parked time included; checked once a minute). At least 1000;
    /// defaults to the worker's `default_pending_timeout_ms`. Fanout copies use
    /// the node's value.
    timeout_ms: Option<u64>,
    /// Refires after a timeout, each in a new session; a failed turn is never
    /// retried. At most 10; the default refires once, and a refire repeats the
    /// node's side effects. `0` fails the node at its first timeout.
    retries: Option<u32>,
    /// `"fail"` (default): a failed or cancelled node fails the run when the
    /// output needs it. `"continue"`: it counts as finished with a `null`
    /// result, so readers still run and the run can complete; the error stays in
    /// `node_errors`. Retries run first; a run abort still cancels everything.
    on_error: Option<OnError>,
}

/// A node id, or an array of node ids.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum NodeRefs {
    One(String),
    Many(Vec<String>),
}

impl NodeRefs {
    fn ids(&self) -> &[String] {
        match self {
            NodeRefs::One(id) => std::slice::from_ref(id),
            NodeRefs::Many(ids) => ids,
        }
    }
}

/// `"<id>.<path to an array>"`, or the array itself.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
enum Fanout {
    Path(String),
    Items(Vec<Value>),
}

impl JsonSchema for StartRequest {
    fn schema_name() -> String {
        "StartRequest".to_string()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        RequestWire::json_schema(generator)
    }
}

/// Copy-pasteable shape appended to every shape and parse error.
const SHAPE_HINT: &str =
    "Shape: {\"nodes\":{\"a\":{\"task\":\"<what a does>\"},\"b\":{\"task\":\"<what b \
    does>\",\"reads\":\"a\"}},\"input\":<any JSON>}.\n\
    Node keys: task (required), reads, fanout, schema, model, profile, provider, system_prompt, \
    functions, filesystem_root, loop, label, phase, timeout_ms, retries, on_error.\n\
    Top-level keys: nodes (required), input, output, max_concurrency, default_functions, \
    idempotency_key, notify.\n\
    Omit `model` to run a node on your own model. Omit `output` to return every node no other \
    node reads.";

// The hook stamps the two `caller_*` keys, so they are known but not advertised.
const TOP_KEYS: &[&str] = &[
    "nodes",
    "input",
    "output",
    "max_concurrency",
    "default_functions",
    "idempotency_key",
    "notify",
    "caller_session_id",
    "caller_fs_root",
];
const NODE_KEYS: &[&str] = &[
    "task",
    "reads",
    "fanout",
    "schema",
    "model",
    "profile",
    "provider",
    "system_prompt",
    "functions",
    "filesystem_root",
    "loop",
    "label",
    "phase",
    "timeout_ms",
    "retries",
    "on_error",
];
const MAX_LISTED_IDS: usize = 20;

const DEFINITION_REMOVED: &str = "`definition` was removed: put `nodes`, `output`, \
    `max_concurrency` and `default_functions` at the top level";
const AGENT_OBJECT_REMOVED: &str = "`agent` was removed: put `model`, `profile`, `provider`, \
    `system_prompt`, `functions` and `filesystem_root` on the node; `agent.output` \
    {\"type\":\"json\",\"schema\":S} is now \"schema\": S";
const AGENT_STRING_REMOVED: &str = "`agent` was removed: a profile id goes in \"profile\":\"x\"";
const INPUT_REMOVED: &str = "a node has no `input`. The run's `input` sits beside `nodes` and \
    reaches every node without `reads`; node-specific text goes in `task`. (Old shape: \
    `input.template` is `task`, `input.from` \"node:x\" is \"reads\":\"x\".)";
const DEPENDS_ON_REMOVED: &str =
    "`depends_on` is not a node field: `reads` orders the nodes and delivers the result";
const NODE_OUTPUT: &str = "`output` is not a node field: a JSON result takes \"schema\": <JSON \
    Schema>; the run's `output` sits beside `nodes`";

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

/// The JSON types a known key accepts (as `json_type` names them) and how a
/// message describes them. `None` for a key of any type.
fn key_type(key: &str) -> Option<(&'static [&'static str], &'static str)> {
    const STRING: &[&str] = &["a string"];
    const IDS: &[&str] = &["a string", "an array"];
    const POLICY: &[&str] = &["a string", "an array", "an object"];
    const OBJECT: &[&str] = &["an object"];
    const NUMBER: &[&str] = &["a number"];
    Some(match key {
        "task" | "model" | "provider" | "profile" | "system_prompt" | "filesystem_root"
        | "label" | "phase" | "idempotency_key" | "caller_session_id" | "caller_fs_root" => {
            (STRING, "a string")
        }
        "reads" | "output" => (IDS, "a node id or an array of ids"),
        "fanout" => (IDS, "\"<id>.<path to an array>\" or an array of values"),
        "schema" => (OBJECT, "a JSON Schema object"),
        "functions" | "default_functions" => (
            POLICY,
            "an allow-list array, a string, or {\"allow\":[…],\"deny\":[…]}",
        ),
        "loop" | "notify" => (OBJECT, "an object"),
        "timeout_ms" | "retries" | "max_concurrency" => (NUMBER, "a non-negative integer"),
        "on_error" => (STRING, "\"fail\" or \"continue\""),
        _ => return None,
    })
}

/// `None` when `val` has a JSON type `key` accepts, else the message tail.
fn type_problem(key: &str, val: &Value) -> Option<String> {
    let (types, expected) = key_type(key)?;
    let found = json_type(val);
    if !types.contains(&found) {
        return Some(format!("`{key}` must be {expected}, not {found}"));
    }
    // Only the integer keys accept a number; the typed parse would refuse -1 or 1.5
    // without naming the key.
    if found == "a number" && val.as_u64().is_none() {
        return Some(format!("`{key}` must be {expected}, not {val}"));
    }
    let ids = matches!(key, "reads" | "output");
    let bad = val
        .as_array()
        .and_then(|a| a.iter().find(|e| ids && !e.is_string()))?;
    Some(format!(
        "`{key}` must be {expected}, not an array holding {}",
        json_type(bad)
    ))
}

/// An object that holds a node key: a node written in the wrong place.
fn looks_like_node(v: &Value) -> bool {
    v.as_object().is_some_and(|o| {
        ["task", "reads", "agent", "input"]
            .iter()
            .any(|k| o.contains_key(*k))
    })
}

fn list_ids(ids: &[&str]) -> String {
    let shown = ids[..ids.len().min(MAX_LISTED_IDS)].join(", ");
    match ids.len().saturating_sub(MAX_LISTED_IDS) {
        0 => shown,
        more => format!("{shown} and {more} more"),
    }
}

/// A removed-key recipe, said once for every node that needs it.
#[derive(Default)]
struct Recipes(Vec<(&'static str, Vec<String>)>);

impl Recipes {
    fn add(&mut self, recipe: &'static str, node: &str) {
        match self.0.iter_mut().find(|(r, _)| *r == recipe) {
            Some((_, nodes)) => nodes.push(node.to_string()),
            None => self.0.push((recipe, vec![node.to_string()])),
        }
    }

    fn report(self, p: &mut Vec<String>) {
        for (recipe, nodes) in self.0 {
            p.push(match nodes.len() {
                1 => format!("{recipe} (node `{}`)", nodes[0]),
                n => format!("{recipe} (node `{}` and {} more)", nodes[0], n - 1),
            });
        }
    }
}

/// Stage A: collect EVERY shape problem of the raw request in one pass, so a
/// caller fixes them all at once. The typed parse and `validate_def` run after.
fn collect_problems(v: &Value) -> Vec<String> {
    let mut p = Vec::new();
    let Some(top) = v.as_object() else {
        p.push(format!(
            "the request must be an object {{\"nodes\":{{…}}}}, not {}",
            json_type(v)
        ));
        return p;
    };
    // First: the lenient parse would otherwise leave the agent waiting for a reply
    // that never comes.
    if top.get("reply_to").is_some() {
        p.push(
            "`reply_to` was removed: drop it, then make the `harness::observe` call this \
             function's `next` gives and end your turn (workers: `notify`)"
                .to_string(),
        );
    }
    let mut level: BTreeMap<&str, &Value> = top
        .iter()
        .filter(|(k, v)| !v.is_null() && k.as_str() != "reply_to")
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    // Check inside a removed wrapper too, so one refusal names every moved key.
    let mut wrapped = false;
    for wrapper in ["definition", "workflow"] {
        let Some(inner) = level.remove(wrapper) else {
            continue;
        };
        if !wrapped {
            p.push(DEFINITION_REMOVED.to_string());
            wrapped = true;
        }
        for (k, v) in inner.as_object().into_iter().flatten() {
            if !v.is_null() {
                level.entry(k.as_str()).or_insert(v);
            }
        }
    }

    let mut typed_ok: BTreeSet<&str> = BTreeSet::new();
    for (&k, &val) in &level {
        // The engine adds its own `_`-prefixed metadata (`_caller_worker_id`) to every
        // payload; it is not part of the request.
        if k.starts_with('_') {
            continue;
        }
        if k == "version" {
            p.push("`version` was removed: drop it".to_string());
        } else if TOP_KEYS.contains(&k) {
            if k == "output" && val.get("from").is_some() {
                p.push(
                    "`output` is a node id or an array of ids, for example \"output\":\"report\", \
                     not {\"from\":…}"
                        .to_string(),
                );
            } else if let Some(problem) = type_problem(k, val) {
                p.push(problem);
            } else {
                typed_ok.insert(k);
            }
        } else if looks_like_node(val) {
            p.push(format!(
                "`{k}` looks like a node written beside `nodes`: move it inside `nodes`"
            ));
        } else if NODE_KEYS.contains(&k) {
            p.push(format!(
                "`{k}` is a node field: put it inside a node under `nodes`"
            ));
        } else {
            p.push(format!("unknown top-level field `{k}`"));
        }
    }

    let mut ids: Vec<&str> = Vec::new();
    let mut nodes: Vec<(&str, &Value)> = Vec::new();
    match level.get("nodes") {
        None => p.push(
            "missing `nodes`: an object keyed by node id, {\"<id>\":{\"task\":\"…\"}}".to_string(),
        ),
        Some(Value::Object(n)) if n.is_empty() => {
            p.push("`nodes` is empty: add at least one node".to_string())
        }
        Some(Value::Object(n)) => {
            for (id, node) in n {
                if id.contains(['#', '.', '/']) {
                    p.push(format!("node id `{id}` must not contain `#`, `.` or `/`"));
                } else if TOP_KEYS.contains(&id.as_str()) && node.get("task").is_none() {
                    p.push(format!(
                        "`{id}` is a top-level field written inside `nodes`: close `nodes` first"
                    ));
                } else {
                    ids.push(id);
                    nodes.push((id, node));
                }
            }
        }
        Some(other) => p.push(format!(
            "`nodes` must be an object keyed by node id, not {}",
            json_type(other)
        )),
    }

    let mut recipes = Recipes::default();
    for (id, node) in nodes {
        collect_node_problems(id, node, &ids, &mut recipes, &mut p);
    }
    recipes.report(&mut p);

    if typed_ok.contains("output") {
        let out = level["output"];
        if out.as_array().is_some_and(Vec::is_empty) {
            p.push(
                "`output` must name a node; omit it to return every node no other node reads"
                    .to_string(),
            );
        }
        for id in out
            .as_array()
            .map_or_else(|| vec![out], |a| a.iter().collect())
            .into_iter()
            .filter_map(Value::as_str)
        {
            if let Some(bare) = id.strip_prefix("node:") {
                p.push(format!("`output`: write the bare id `{bare}`, not `{id}`"));
            } else if !ids.is_empty() && !ids.contains(&id) {
                p.push(format!(
                    "`output` names `{id}`, which is not a node (nodes: {})",
                    list_ids(&ids)
                ));
            }
        }
    }
    p
}

/// The run input named where a node is expected (`input`, `run_input`, or a
/// path into them), when no node has that id.
fn names_run_input(entry: &str, ids: &[&str]) -> bool {
    let base = entry.split('.').next().unwrap_or(entry);
    (matches!(base, "input" | "run_input") || entry == "workflow.input") && !ids.contains(&base)
}

fn collect_node_problems(
    id: &str,
    node: &Value,
    ids: &[&str],
    recipes: &mut Recipes,
    p: &mut Vec<String>,
) {
    let Some(n) = node.as_object() else {
        p.push(format!(
            "node `{id}` must be an object {{\"task\":\"…\"}}, not {}",
            json_type(node)
        ));
        return;
    };
    let get = |k: &str| n.get(k).filter(|v| !v.is_null());
    let mut typed_ok: BTreeSet<&str> = BTreeSet::new();
    for (k, val) in n.iter().filter(|(_, v)| !v.is_null()) {
        let k = k.as_str();
        match k {
            "agent" if val.is_string() => recipes.add(AGENT_STRING_REMOVED, id),
            "agent" => recipes.add(AGENT_OBJECT_REMOVED, id),
            "input" => recipes.add(INPUT_REMOVED, id),
            "depends_on" => recipes.add(DEPENDS_ON_REMOVED, id),
            "output" => recipes.add(NODE_OUTPUT, id),
            "max_concurrency" | "idempotency_key" | "notify" => p.push(format!(
                "node `{id}`: `{k}` is a top-level field: put it beside `nodes`"
            )),
            "default_functions" => p.push(format!(
                "node `{id}`: on a node the field is `functions`; `default_functions` sits \
                 beside `nodes`"
            )),
            "task" => {}
            "fanout" if val.get("over").is_some() => p.push(format!(
                "node `{id}`: `fanout` is a string, \"<id>.<path to an array>\", or an array of \
                 values, not {{\"over\":…}}"
            )),
            _ if NODE_KEYS.contains(&k) => match type_problem(k, val) {
                Some(problem) => p.push(format!("node `{id}`: {problem}")),
                None => {
                    typed_ok.insert(k);
                }
            },
            _ if looks_like_node(val) => p.push(format!(
                "node `{id}`: `{k}` looks like a node written inside `{id}`; close `{id}` first: \
                 nodes are siblings under `nodes`"
            )),
            _ => p.push(format!("node `{id}`: unknown field `{k}`")),
        }
    }

    let has_task = get("task")
        .and_then(Value::as_str)
        .is_some_and(|t| !t.trim().is_empty());
    if !has_task && get("input").is_none() {
        p.push(format!(
            "node `{id}`: missing `task`, the text that tells this node what to do"
        ));
    }

    if typed_ok.contains("reads") {
        let reads = get("reads").unwrap();
        let entries: Vec<&str> = match reads.as_array() {
            Some(a) => a.iter().filter_map(Value::as_str).collect(),
            None => reads.as_str().into_iter().collect(),
        };
        if reads.as_array().is_some_and(Vec::is_empty) {
            p.push(format!(
                "node `{id}`: `reads` is an empty array: name a node, or omit `reads` to receive \
                 the run `input`"
            ));
        }
        for entry in entries {
            let base = entry.split('.').next().unwrap_or(entry);
            if let Some(bare) = entry.strip_prefix("node:") {
                p.push(format!(
                    "node `{id}`: write the bare id `{bare}`, not `{entry}`"
                ));
            } else if names_run_input(entry, ids) {
                p.push(run_input_problem(id));
            } else if entry.contains('.') && ids.contains(&base) {
                p.push(format!(
                    "node `{id}`: `reads` takes a whole result; only `fanout` takes a path. \
                     Write `{base}`"
                ));
            } else if !ids.contains(&entry) {
                p.push(format!(
                    "node `{id}`: `reads` names `{entry}`, which is not a node (nodes: {}). Omit \
                     `reads` to receive the run `input`",
                    list_ids(ids)
                ));
            }
        }
    }

    if typed_ok.contains("fanout") {
        match get("fanout").unwrap() {
            Value::Array(items) if items.is_empty() => p.push(format!(
                "node `{id}`: `fanout` is an empty array, so no copy would run: list at least one \
                 element, or drop `fanout`"
            )),
            Value::String(path) if path.trim_start().starts_with('[') => p.push(format!(
                "node `{id}`: `fanout` holds an array written as a string: write the array itself"
            )),
            Value::String(path) => {
                let base = path.split('.').next().unwrap_or(path);
                if let Some(bare) = path.strip_prefix("node:") {
                    p.push(format!(
                        "node `{id}`: write the bare id `{bare}`, not `{path}`"
                    ));
                } else if names_run_input(path, ids) {
                    p.push(run_input_problem(id));
                } else if !ids.contains(&base) {
                    p.push(format!(
                        "node `{id}`: `fanout` names `{base}`, which is not a node (nodes: {})",
                        list_ids(ids)
                    ));
                }
            }
            _ => {}
        }
    }

    if typed_ok.contains("schema") {
        match get("schema").unwrap().get("type").and_then(Value::as_str) {
            Some("json") => p.push(format!(
                "node `{id}`: `schema` is the JSON Schema itself: use the inner `schema` value, or \
                 {{\"type\":\"object\"}} for any object"
            )),
            Some("text") => p.push(format!("node `{id}`: omit `schema` for a text node")),
            _ => {}
        }
    }

    // `lower` reads a blank `model` or `provider` as omitted, so they count as omitted here too.
    let no_model = get("model").is_none_or(|m| m.as_str().is_some_and(|m| m.trim().is_empty()));
    let has_provider =
        get("provider").is_some_and(|v| v.as_str().is_none_or(|s| !s.trim().is_empty()));
    if has_provider && no_model && get("profile").is_none() {
        p.push(format!(
            "node `{id}`: `provider` needs `model`; name the model, or drop `provider` to run on \
             your own model"
        ));
    }
}

fn run_input_problem(id: &str) -> String {
    format!(
        "node `{id}`: the run `input` is not a node. A node without `reads` or `fanout` receives \
         it; `reads` takes node results, and `fanout` takes a node's array or an array written in \
         place"
    )
}

fn format_problems(problems: &[String]) -> String {
    let n = problems.len();
    let mut s = format!(
        "workflow::start: {n} problem{}:",
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

impl<'de> Deserialize<'de> for StartRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;
        let v = Value::deserialize(deserializer)?;
        let problems = collect_problems(&v);
        if !problems.is_empty() {
            return Err(D::Error::custom(format_problems(&problems)));
        }
        serde_json::from_value::<RequestWire>(v)
            .map(lower)
            .map_err(|e| D::Error::custom(format!("workflow::start: {e}.\n{SHAPE_HINT}")))
    }
}

/// Build the stored definition from a request that passed the shape pass. Pure.
fn lower(w: RequestWire) -> StartRequest {
    fn node_ref(id: &str) -> String {
        format!("node:{id}")
    }
    // Nodes another node names in `reads` or `fanout`; the rest are the default output.
    let mut read: BTreeSet<&str> = BTreeSet::new();
    for (id, n) in &w.nodes {
        let fanout_src = match &n.fanout {
            Some(Fanout::Path(path)) => path.split('.').next(),
            _ => None,
        };
        let reads = n.reads.iter().flat_map(NodeRefs::ids).map(String::as_str);
        read.extend(reads.chain(fanout_src).filter(|r| *r != id.as_str()));
    }
    let unread: Vec<&String> = w
        .nodes
        .keys()
        .filter(|id| !read.contains(id.as_str()))
        .collect();
    let output = match &w.output {
        Some(NodeRefs::One(id)) => OutputFrom::One(node_ref(id)),
        Some(NodeRefs::Many(ids)) => OutputFrom::Many(ids.iter().map(|id| node_ref(id)).collect()),
        None => {
            // Every node is read: the reads form a cycle. Name the nodes outside any
            // loop body, so `validate_def` reports the cycle or the loop, not `output`.
            let in_body: BTreeSet<&String> = w
                .nodes
                .values()
                .filter_map(|n| n.loop_spec.as_ref())
                .flat_map(|l| &l.body)
                .collect();
            let mut out = unread;
            if out.is_empty() {
                out = w.nodes.keys().filter(|id| !in_body.contains(id)).collect();
            }
            if out.is_empty() {
                out = w.nodes.keys().collect();
            }
            match out.as_slice() {
                [one] => OutputFrom::One(node_ref(one)),
                many => OutputFrom::Many(many.iter().map(|id| node_ref(id)).collect()),
            }
        }
    };

    let nodes = w
        .nodes
        .into_iter()
        .map(|(id, n)| {
            let from = match (&n.reads, &n.fanout) {
                (Some(NodeRefs::One(r)), _) => InputFrom::One(node_ref(r)),
                (Some(NodeRefs::Many(rs)), _) => {
                    InputFrom::Many(rs.iter().map(|r| node_ref(r)).collect())
                }
                (None, Some(_)) => "fanout_item".into(),
                (None, None) => "run_input".into(),
            };
            let fanout = n.fanout.map(|f| match f {
                Fanout::Path(path) => FanoutSpec {
                    over: format!("node:{path}"),
                    items: None,
                },
                Fanout::Items(items) => FanoutSpec {
                    over: String::new(),
                    items: Some(items),
                },
            });
            let node = NodeDef {
                agent: AgentSpec {
                    model: n.model.filter(|m| !m.trim().is_empty()).unwrap_or_default(),
                    profile: n.profile,
                    provider: n.provider.filter(|p| !p.trim().is_empty()),
                    system_prompt: n.system_prompt,
                    functions: n.functions,
                    output: n.schema.map(|s| json!({ "type": "json", "schema": s })),
                    filesystem_root: n.filesystem_root,
                },
                input: InputSpec {
                    from,
                    template: Some(n.task),
                },
                depends_on: Vec::new(),
                fanout,
                timeout_ms: n.timeout_ms,
                retries: n.retries,
                on_error: n.on_error.unwrap_or_default(),
                label: n.label,
                phase: n.phase,
                loop_spec: n.loop_spec,
            };
            (id, node)
        })
        .collect();

    let mut definition = WorkflowDef {
        version: SUPPORTED_DEF_VERSION,
        nodes,
        output: OutputRef { from: output },
        default_functions: w.default_functions,
        max_concurrency: w.max_concurrency,
    };
    add_read_dependencies(&mut definition);
    StartRequest {
        definition,
        input: w.input,
        idempotency_key: w.idempotency_key,
        notify: w.notify,
        caller_session_id: w.caller_session_id,
        caller_fs_root: w.caller_fs_root,
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StartResponse {
    pub run_id: String,
    /// For an agent caller: the `harness::observe` call that delivers the run's
    /// outcome to this session. Make it exactly as given, then end the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<Value>,
}

/// How long the `next` wake waits: an hour under the run's retention, so a wake
/// that expires always finds its run still stored.
const NEXT_EXPIRES_IN_MS: i64 = super::sweep::TERMINAL_RETENTION_MS - 3_600_000;

/// The start response; `next` only for a hook-stamped agent caller. Pure.
fn start_response(run_id: String, caller_session_id: Option<&str>) -> StartResponse {
    let next = caller_session_id.map(|_| {
        json!({
            "call": "harness::observe",
            "arguments": {
                "function_id": crate::events::ON_RUN_COMPLETED_FN,
                "args": { "run_id": run_id },
                "label": format!("workflow {run_id}"),
                "lifecycle": { "expires_in_ms": NEXT_EXPIRES_IN_MS },
            },
            "then": format!(
                "END YOUR TURN after this call; do not poll workflow::status. The outcome \
                 arrives as `[notification] workflow {run_id}: {{…}}`, at once if the run \
                 already ended. Text ending in `…(truncated)`, or an array ending in an \
                 `…N more entries omitted` string, is incomplete: call workflow::status once \
                 for this run. A second notification for this run is a duplicate: ignore it. \
                 If the observe is refused with UNKNOWN_RUN, the run was collected: say so \
                 and do not retry. If this call was never made (an interrupted turn), make it \
                 now. If the wake expires unfired, make this call again as given instead of \
                 reading workflow::status: a finished run replays at once."
            ),
        })
    });
    StartResponse { run_id, next }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Fold each node's `input.from` / `fanout.over` `"node:<id>"` reads into its
/// `depends_on` (deduped). A read is a data dependency, but scheduling consults only
/// `depends_on` (see `dag::deps_done` / `ready_frontier`); aligning them here means a
/// node never fires before a node it reads, and keeps `validate_def`'s cycle check
/// honest about the graph that actually executes. Run BEFORE `validate_def`. Idempotent.
fn add_read_dependencies(def: &mut WorkflowDef) {
    for node in def.nodes.values_mut() {
        let mut refs: Vec<String> = Vec::new();
        for src in node.input.from.sources() {
            if let Some(rest) = src.strip_prefix("node:") {
                refs.push(rest.split('.').next().unwrap_or(rest).to_string());
            }
        }
        if let Some(fanout) = &node.fanout {
            if let Some(rest) = fanout.over.strip_prefix("node:") {
                refs.push(rest.split('.').next().unwrap_or(rest).to_string());
            }
        }
        for r in refs {
            if !node.depends_on.contains(&r) {
                node.depends_on.push(r);
            }
        }
    }
}

// Mirrors iii-directory's `validate_name`, the rule `directory::agents::get` applies
// to the id the harness resolves, so a malformed profile fails here and not mid-run.
fn is_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
}

/// Validate a lowered `WorkflowDef` for structural correctness. Messages use the
/// request's names (`schema`, `reads`, `fanout`, `output`), not the stored ones.
///
/// Rules:
/// 0. `version` must be supported; node count is bounded; node ids must not
///    contain the reserved separators `#`/`.`; `output.from` must name one or
///    more distinct existing nodes.
/// 1. A profile must be a profile id and excludes `agent.system_prompt`; `timeout_ms`
///    is at least 1000, `retries` at most 10, `max_concurrency` at least 1; a
///    `label` or `phase` is non-empty and at most 80 characters. A missing model is
///    the model step's concern (`handle`).
/// 2. Any node referenced by another node's `fanout.over` (strip `node:`,
///    take the part before the first `.`) MUST have `agent.output` be a JSON
///    object with `"type" == "json"`. A node read by `input.from` may be text.
/// 3. The `depends_on` graph must be acyclic (and reference existing nodes).
/// 6. A `loop` is well formed (`validate_loops`): `max_repeats` 1 to 9, a JSON loop
///    node whose declared schema has `while.field` (and `while.equals` in its enum),
///    a body of distinct other nodes that all feed the loop node, at most one loop
///    per node, no fanout in a loop, and nothing outside the loop reads a body node.
///    Runs before Rule 3 and reads `depends_on` after `add_read_dependencies`.
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
            "the workflow has {} nodes (max {})",
            def.nodes.len(),
            MAX_NODES
        )));
    }

    // Rule 0c: node ids must not contain the reserved separators '#' (fanout item
    // index), '.' (over-path), or '/' (the run_id/node_uid storage-key separator —
    // see ids::node_result_key), any of which would mis-parse at dispatch time or
    // corrupt result-key composition.
    for node_id in def.nodes.keys() {
        if node_id.contains('#') || node_id.contains('.') || node_id.contains('/') {
            return Err(WorkflowError::InvalidDef(format!(
                "node id '{}' must not contain '#', '.', or '/'",
                node_id
            )));
        }
    }

    // Rule 0d: every output node must reference an existing node, otherwise the
    // run can never reach Completed and hangs in AwaitingNodes forever.
    let out_ids = def.output.from.node_ids();
    if out_ids.is_empty() {
        return Err(WorkflowError::InvalidDef(
            "output must name at least one node".into(),
        ));
    }
    let mut seen_out = BTreeSet::new();
    for out_id in out_ids {
        if !def.nodes.contains_key(out_id) {
            return Err(WorkflowError::InvalidDef(format!(
                "output references unknown node '{}'",
                out_id
            )));
        }
        if !seen_out.insert(out_id) {
            return Err(WorkflowError::InvalidDef(format!(
                "output lists node '{}' twice",
                out_id
            )));
        }
    }

    // Rule 1: a profile replaces the system prompt.
    for (node_id, node) in &def.nodes {
        let agent = &node.agent;
        if let Some(profile) = &agent.profile {
            if !is_profile_id(profile) {
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{node_id}': profile {profile:?} is not a profile id \
                     (lowercase letters, digits, '-' and '_', at most 64 characters)"
                )));
            }
            if agent.system_prompt.is_some() {
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{node_id}': profile supplies the system prompt; drop system_prompt or profile"
                )));
            }
        }
    }

    // Rule 1a: scheduling limits.
    if def.max_concurrency == Some(0) {
        return Err(WorkflowError::InvalidDef(
            "max_concurrency must be at least 1".into(),
        ));
    }
    for (id, node) in &def.nodes {
        if node.timeout_ms.is_some_and(|t| t < MIN_NODE_TIMEOUT_MS) {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{id}': timeout_ms must be at least {MIN_NODE_TIMEOUT_MS} (milliseconds)"
            )));
        }
        if node.retries.is_some_and(|r| r > MAX_NODE_RETRIES) {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{id}': retries must be at most {MAX_NODE_RETRIES}"
            )));
        }
        for (key, text) in [("label", &node.label), ("phase", &node.phase)] {
            if text
                .as_ref()
                .is_some_and(|t| t.trim().is_empty() || t.chars().count() > MAX_LABEL_CHARS)
            {
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{id}': {key} must be non-empty and at most {MAX_LABEL_CHARS} characters"
                )));
            }
        }
    }

    // Rule 1b: functions, if present, must be a shape the worker can coerce
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
                        "node '{}': functions must be an array of function-id strings; \
                         found a non-string element ({})",
                        node_id, bad
                    )));
                }
            }
            Some(other) => {
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{}': functions must be an allow-list array (e.g. \
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
                if let Some(schema) = crate::reconcile::output_schema(output) {
                    if schema.is_object() {
                        let has_type = schema
                            .get("type")
                            .and_then(|t| t.as_str())
                            .map(|t| !t.trim().is_empty())
                            .unwrap_or(false);
                        if !has_type {
                            return Err(WorkflowError::InvalidDef(format!(
                                "node '{}': schema has no top-level \"type\" — an \
                                 empty/typeless schema is rejected by the provider (\"input_schema.type: Field required\"). Use a real JSON \
                                 Schema, e.g. {{\"type\":\"object\",\"properties\":{{...}}}}.",
                                node_id
                            )));
                        }
                    }
                    // The model cannot repair a schema that does not compile; every answer would be denied.
                    if let Err(e) = jsonschema::JSONSchema::compile(schema) {
                        return Err(WorkflowError::InvalidDef(format!(
                            "node '{node_id}': schema is not a valid JSON Schema: {e}"
                        )));
                    }
                }
            }
        }
    }

    // Collect nodes that must have JSON output. `input.from` reads a node whole, so
    // a text node there arrives as one JSON string; an unknown `input.from` ref is
    // caught by Rule 3 once `add_read_dependencies` folded it into `depends_on`.
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
                "node '{dep_id}' is read by a fanout but has no `schema`; a fanout walks a JSON \
                 value to an array (`reads` may read a text node whole)"
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
                let path = fanout.over.strip_prefix("node:").unwrap_or(&fanout.over);
                let dep = path.split('.').next().unwrap_or(path);
                return Err(WorkflowError::InvalidDef(format!(
                    "node '{node_id}': fanout \"{path}\" resolves to a {leaf_type} in node \
                     '{dep}'s declared schema, but a fanout iterates an ARRAY. Point `fanout` at a \
                     field that node '{dep}'s schema declares as {{\"type\":\"array\"}}. (To run \
                     several agents over one value, for example several critics over one draft, \
                     write the angles as an array in `fanout` and name the value in `reads`: \
                     {{\"fanout\":[\"security\",\"cost\"],\"reads\":\"draft\"}}.)"
                )));
            }
        }
    }

    // Rule 6, before Rule 3: a body node reading its loop node gets the loop error.
    validate_loops(def).map_err(WorkflowError::InvalidDef)?;

    // Rule 3: no cycles.
    crate::dag::validate_acyclic(def).map_err(WorkflowError::InvalidDef)?;

    Ok(())
}

/// Rule 6: the `loop` rules (6a to 6o). Safe on a graph with cycles or unknown
/// dependencies (Rule 3 reports those). Pure.
fn validate_loops(def: &WorkflowDef) -> Result<(), String> {
    // node id → the loop node of its loop (6i: at most one).
    let mut group: BTreeMap<&str, &str> = BTreeMap::new();
    for (id, node) in &def.nodes {
        let Some(spec) = &node.loop_spec else {
            continue;
        };
        if !(1..=MAX_LOOP_REPEATS).contains(&spec.max_repeats) {
            return Err(format!(
                "node '{id}': loop.max_repeats must be between 1 and {MAX_LOOP_REPEATS}"
            ));
        }
        let field = &spec.cond.field;
        let segs: Vec<&str> = field.split('.').collect();
        if segs.iter().any(|s| s.is_empty()) {
            return Err(format!(
                "node '{id}': loop.while.field must be a dotted path without empty segments; got \"{field}\""
            ));
        }
        let equals = &spec.cond.equals;
        if equals.is_empty() {
            return Err(format!("node '{id}': loop.while.equals must not be empty"));
        }
        let output = node.agent.output.as_ref();
        if output.and_then(|o| o.get("type")).and_then(Value::as_str) != Some("json") {
            return Err(format!(
                "node '{id}': loop.while reads this node's result, so the node needs a `schema`"
            ));
        }
        if let Some(schema) = output
            .and_then(|o| o.get("schema"))
            .filter(|s| s.is_object())
        {
            match schema_at(schema, &segs) {
                Err(Some(seg)) => {
                    return Err(format!(
                        "node '{id}': loop.while.field = \"{field}\" is not declared in the node's \
                         schema (missing segment '{seg}')"
                    ))
                }
                Ok(leaf) => {
                    // `dag::condition_holds` compares strings only, so a non-string leaf never holds.
                    let non_string = match leaf.get("type") {
                        _ if schema_is_dynamic(leaf) => None,
                        Some(Value::String(t)) if t != "string" => Some(t.clone()),
                        Some(Value::Array(types)) if !types.iter().any(|t| t == "string") => Some(
                            types
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("|"),
                        ),
                        _ => None,
                    };
                    if let Some(t) = non_string {
                        return Err(format!(
                            "node '{id}': loop.while.field = \"{field}\" is declared as {t}; the \
                             loop compares strings only"
                        ));
                    }
                    if let Some(values) = leaf.get("enum").and_then(Value::as_array) {
                        if !values.iter().any(|v| v.as_str() == Some(equals.as_str())) {
                            let listed: Vec<String> = values
                                .iter()
                                .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
                                .collect();
                            return Err(format!(
                                "node '{id}': loop.while.equals = \"{equals}\" is not one of the \
                                 values the schema declares for '{field}' ({})",
                                listed.join(", ")
                            ));
                        }
                    }
                }
                Err(None) => {}
            }
            if let Some(seg) = unrequired_segment(schema, &segs) {
                let at = if segs.len() > 1 {
                    format!(" (segment '{seg}')")
                } else {
                    String::new()
                };
                return Err(format!(
                    "node '{id}': loop.while.field = \"{field}\" must be listed in the schema's \
                     `required`{at}; a result without it ends the loop after one pass"
                ));
            }
        }
        let mut seen = BTreeSet::new();
        for b in &spec.body {
            if !def.nodes.contains_key(b) {
                return Err(format!(
                    "node '{id}': loop.body references unknown node '{b}'"
                ));
            }
            if b == id || !seen.insert(b.as_str()) {
                return Err(format!(
                    "node '{id}': loop.body must list other nodes once each; got '{b}'"
                ));
            }
        }
        for m in std::iter::once(id).chain(&spec.body) {
            if let Some(other) = group.insert(m.as_str(), id.as_str()) {
                return Err(format!(
                    "node '{m}' belongs to the loops of '{other}' and '{id}'; a node belongs to \
                     at most one loop"
                ));
            }
        }
    }
    for (m, c) in &group {
        if def.nodes[*m].fanout.is_some() {
            return Err(format!(
                "node '{m}': a node of the loop '{c}' must not declare fanout"
            ));
        }
    }
    for (id, node) in &def.nodes {
        let Some(spec) = &node.loop_spec else {
            continue;
        };
        // 6k: walk back from the loop node through body nodes only.
        let mut fed: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<&str> = vec![id.as_str()];
        while let Some(n) = stack.pop() {
            for dep in &def.nodes[n].depends_on {
                if spec.body.contains(dep) && fed.insert(dep.as_str()) {
                    stack.push(dep.as_str());
                }
            }
        }
        if let Some(b) = spec.body.iter().find(|b| !fed.contains(b.as_str())) {
            return Err(format!(
                "node '{id}': loop.body node '{b}' does not feed '{id}'; '{id}' must read every \
                 body node, directly or through other body nodes"
            ));
        }
    }
    let is_body = |n: &str| group.get(n).is_some_and(|c| *c != n);
    for (x, node) in &def.nodes {
        for dep in &node.depends_on {
            let Some(c) = group.get(dep.as_str()) else {
                continue;
            };
            if group.get(x.as_str()) == Some(c) {
                if dep == c && x != c {
                    return Err(format!(
                        "node '{x}' reads '{c}', the node that carries its loop; the previous \
                         result arrives as `loop_previous`"
                    ));
                }
            } else if is_body(dep) {
                return Err(format!(
                    "node '{x}' reads '{dep}', a loop.body node of '{c}'; outside the loop read \
                     '{c}', which returns the last pass"
                ));
            }
        }
    }
    for out in def.output.from.node_ids() {
        if is_body(out) {
            return Err(format!(
                "output references '{out}', a loop.body node of '{}'; name '{}' instead",
                group[out], group[out]
            ));
        }
    }
    for (x, node) in &def.nodes {
        let over_dep = node
            .fanout
            .as_ref()
            .and_then(|f| f.over.strip_prefix("node:"))
            .map(|r| r.split('.').next().unwrap_or(r));
        if let Some(c) =
            over_dep.filter(|d| def.nodes.get(*d).is_some_and(|n| n.loop_spec.is_some()))
        {
            return Err(format!(
                "node '{x}': fanout reads '{c}', which carries a loop; a fanout cannot \
                 iterate a loop's result"
            ));
        }
    }
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

    proven_nonarray_type(schema_at(schema, walk).ok()?)
}

/// Walk `segs` through a declared schema's `properties`: `Ok(leaf)`, `Err(Some(seg))`
/// when a static object schema does not declare `seg`, `Err(None)` when the shape
/// cannot be proven (a dynamic schema, or no `properties` to check).
fn schema_at<'a>(schema: &'a Value, segs: &[&str]) -> Result<&'a Value, Option<String>> {
    let mut cur = schema;
    for seg in segs {
        if schema_is_dynamic(cur) {
            return Err(None);
        }
        let props = cur.get("properties").ok_or(None)?;
        cur = props.get(*seg).ok_or_else(|| Some(seg.to_string()))?;
    }
    Ok(cur)
}

/// The first of `segs` that its enclosing object schema does not list in
/// `required`. None when every one is listed, when a level uses `anyOf`, `oneOf`,
/// `allOf` or `$ref`, or when the schema stops declaring the path.
fn unrequired_segment<'a>(schema: &Value, segs: &[&'a str]) -> Option<&'a str> {
    let mut cur = schema;
    for seg in segs {
        if ["anyOf", "oneOf", "allOf", "$ref"]
            .iter()
            .any(|k| cur.get(*k).is_some())
        {
            return None;
        }
        let required = cur.get("required").and_then(Value::as_array);
        if !required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some(*seg))) {
            return Some(seg);
        }
        cur = cur.get("properties").and_then(|p| p.get(*seg))?;
    }
    None
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
        .call(iii_sdk::CallRequest {
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

/// The caller's model, for nodes that omit `model`.
#[derive(Debug, Clone, PartialEq)]
enum CallerModel {
    /// No hook-stamped caller session (a worker or CLI caller).
    NoSession,
    /// `harness::status` failed, or its snapshot carried no model.
    Unreadable,
    Known {
        model: String,
        provider: Option<String>,
    },
}

/// A node that runs on the caller's model: `model` and `provider` both omitted.
fn inherits_model(node: &NodeDef) -> bool {
    node.agent.model.is_empty() && node.agent.provider.is_none()
}

/// Write the caller's model into every node that inherits it, so retries, loop
/// passes and fanout copies stay on it. A profile node takes the model but not the
/// provider: the profile's pinned model could otherwise be routed to the wrong
/// provider. Returns the nodes without a profile left without a model. Pure.
fn inherit_caller_model(def: &mut WorkflowDef, caller: &CallerModel) -> Vec<String> {
    let mut missing = Vec::new();
    for (id, node) in def.nodes.iter_mut().filter(|(_, n)| inherits_model(n)) {
        match caller {
            CallerModel::Known { model, provider } => {
                node.agent.model = model.clone();
                if node.agent.profile.is_none() {
                    node.agent.provider = provider.clone();
                }
            }
            _ if node.agent.profile.is_some() => {}
            _ => missing.push(id.clone()),
        }
    }
    missing
}

/// `harness::status {verbose: true}` → the session's `context.model` and
/// `context.provider`. Pure.
fn caller_model_from_status(resp: &Value) -> CallerModel {
    let context = &resp["context"];
    match context["model"].as_str().map(str::trim) {
        Some(model) if !model.is_empty() => CallerModel::Known {
            model: model.to_string(),
            provider: context["provider"]
                .as_str()
                .filter(|p| !p.trim().is_empty())
                .map(str::to_string),
        },
        _ => CallerModel::Unreadable,
    }
}

/// Read the caller session's model; the snapshot exists while its turn runs.
async fn read_caller_model(deps: &Deps, caller_session_id: Option<&str>) -> CallerModel {
    let Some(session_id) = caller_session_id else {
        return CallerModel::NoSession;
    };
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    match deps
        .iii
        .call(iii_sdk::CallRequest {
            function_id: "harness::status".into(),
            payload: json!({ "session_id": session_id, "verbose": true }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
    {
        Ok(resp) => caller_model_from_status(&resp),
        Err(error) => {
            tracing::warn!(%session_id, %error, "caller model unreadable");
            CallerModel::Unreadable
        }
    }
}

/// The model step's refusal, every problem at once: `unknown` written ids, and
/// `missing` nodes left without a model. `registered` is `None` when the catalog
/// could not be read. Pure.
fn model_problems(
    unknown: &[String],
    missing: &[String],
    caller: &CallerModel,
    registered: Option<&BTreeSet<(String, String)>>,
) -> Option<String> {
    let choices = match registered {
        Some(set) => {
            let ids: BTreeSet<&str> = set.iter().map(|(_, id)| id.as_str()).collect();
            format!("one of: {}", ids.into_iter().collect::<Vec<_>>().join(", "))
        }
        None => "an id from router::models::list".to_string(),
    };
    let mut problems = Vec::new();
    if !unknown.is_empty() {
        problems.push(format!(
            "unknown model(s) {unknown:?}: not registered on this engine. Omit `model` to run on \
             your own model, or use {choices}"
        ));
    }
    if !missing.is_empty() {
        let nodes: Vec<String> = missing.iter().map(|id| format!("`{id}`")).collect();
        let why = match caller {
            CallerModel::NoSession => "this call has no caller session to inherit one from",
            _ => "your session's model could not be read",
        };
        problems.push(format!(
            "node(s) {}: no `model`, and {why}; set `model` to {choices}",
            nodes.join(", ")
        ));
    }
    (!problems.is_empty()).then(|| problems.join("; "))
}

/// Check the written model ids, then fill the inherited ones. The order matters:
/// an inherited model runs the caller right now, so it is not checked. Pure.
fn model_step(
    def: &mut WorkflowDef,
    registered: Option<&BTreeSet<(String, String)>>,
    caller: &CallerModel,
) -> Result<(), String> {
    let unknown = registered
        .map(|set| unknown_models(def, set))
        .unwrap_or_default();
    let missing = inherit_caller_model(def, caller);
    model_problems(&unknown, &missing, caller, registered).map_or(Ok(()), Err)
}

/// Every `filesystem_root` must be absolute, without `..`, and lie inside
/// the caller's folder with symlinks resolved on this host, so a symlink inside
/// the caller's folder cannot root a node outside it. Checked once, at start.
fn validate_folders(def: &WorkflowDef, caller_fs_root: Option<&str>) -> Result<(), WorkflowError> {
    use std::path::{Component, Path};
    for (id, node) in &def.nodes {
        let Some(folder) = node.agent.filesystem_root.as_deref() else {
            continue;
        };
        let path = Path::new(folder);
        if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{id}': filesystem_root must be an absolute path without '..', \
                 got {folder:?}"
            )));
        }
        let Some(root) = caller_fs_root else {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{id}': filesystem_root needs the caller's folder to confine it, \
                 and this run has none; start the run from an agent session that has a \
                 folder, or drop filesystem_root"
            )));
        };
        let Ok(caller) = std::fs::canonicalize(root) else {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{id}': filesystem_root cannot be checked: the caller's folder \
                 {root:?} does not exist on the workflow worker's host"
            )));
        };
        // `Path::starts_with` compares whole components, so `/repo2` is not inside `/repo`.
        if !resolve_confined(path).is_some_and(|p| p.starts_with(&caller)) {
            return Err(WorkflowError::InvalidDef(format!(
                "node '{id}': filesystem_root {folder:?} is outside the caller's folder \
                 {root:?}; a node folder must lie inside it"
            )));
        }
    }
    Ok(())
}

/// Where `path` leads, symlinks resolved: its longest existing ancestor
/// canonicalized, then the part that does not exist yet as written. `None` when
/// that part could lead elsewhere once created (a `..`, or a dangling symlink).
// Same rule as the harness's spawn confinement, so a node folder that a run
// creates later (a ticket worktree) is still accepted.
fn resolve_confined(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let (mut cur, mut missing) = (path, Vec::new());
    loop {
        if let Ok(resolved) = std::fs::canonicalize(cur) {
            return Some(missing.iter().rev().fold(resolved, |p, n| p.join(n)));
        }
        match std::fs::symlink_metadata(cur) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return None,
        }
        // `file_name` is None for a trailing `..`.
        missing.push(cur.file_name()?);
        cur = cur.parent()?;
    }
}

// ---------------------------------------------------------------------------
// Enqueue helper
// ---------------------------------------------------------------------------

pub async fn enqueue_tick(
    iii: &iii_sdk::IIIClient,
    run_id: &str,
    step: u64,
) -> Result<(), WorkflowError> {
    enqueue_tick_payload(iii, json!({"run_id": run_id, "step": step})).await
}

/// A wake tick runs even below the run's step floor (see `tick_is_stale`).
pub async fn enqueue_wake_tick(
    iii: &iii_sdk::IIIClient,
    run_id: &str,
    step: u64,
) -> Result<(), WorkflowError> {
    enqueue_tick_payload(iii, json!({"run_id": run_id, "step": step, "wake": true})).await
}

async fn enqueue_tick_payload(
    iii: &iii_sdk::IIIClient,
    payload: Value,
) -> Result<(), WorkflowError> {
    iii.call(iii_sdk::CallRequest {
        function_id: "workflow::tick".into(),
        payload,
        action: Some(iii_sdk::CallAction::Enqueue {
            queue: crate::queue::WORKFLOW_QUEUE.into(),
        }),
        timeout_ms: None,
    })
    .await
    .map_err(|e| WorkflowError::Call(e.to_string()))?;
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

/// One `WorkflowLocks` key serializes every idempotency check-and-set (start)
/// and its GC delete (sweep). Run ids start with `r_`, so it never names a run.
pub(crate) const IDEM_LOCK: &str = "idem";

#[derive(Debug)]
enum IdemHit {
    Hit,
    /// The key names a run that is gone or unreadable; a new start overwrites it.
    Dangling,
    Err(WorkflowError),
}

/// Classify the `get_run` of the run an idempotency key points at. Pure.
fn idem_hit_decision(run: Result<Option<WorkflowRunRecord>, WorkflowError>) -> IdemHit {
    match run {
        Ok(Some(_)) => IdemHit::Hit,
        Ok(None) | Err(WorkflowError::Serde(_)) => IdemHit::Dangling,
        Err(e) => IdemHit::Err(e),
    }
}

/// The run id an idempotency key already started, if that run still exists.
async fn idem_hit(deps: &Deps, key: &str) -> Result<Option<String>, WorkflowError> {
    let Some(run_id) = state::get_idem(&deps.iii, key).await? else {
        return Ok(None);
    };
    match idem_hit_decision(state::get_run(&deps.iii, &run_id).await) {
        IdemHit::Hit => Ok(Some(run_id)),
        IdemHit::Dangling => Ok(None),
        IdemHit::Err(e) => Err(e),
    }
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
/// the caller gets the `run_id` back immediately and receives the outcome through
/// the `next` observe call or `notify` (or by polling `workflow::status`); the
/// harness turn is never blocked.
pub async fn handle(deps: &Deps, mut req: StartRequest) -> Result<StartResponse, WorkflowError> {
    validate_def(&req.definition)?;
    if req
        .caller_fs_root
        .as_deref()
        .is_some_and(|r| r.trim().is_empty())
    {
        return Err(WorkflowError::InvalidDef(
            "caller_fs_root must not be empty; omit it to use the harness default folder".into(),
        ));
    }
    validate_folders(&req.definition, req.caller_fs_root.as_deref())?;

    // The hook-stamped caller/orchestrator session; `None` for a non-agent caller.
    // Used to nest node sessions in the console, to scope the idempotency key
    // (keeping its GET and PUT out of the shared `_anon` bucket, so two callers
    // reusing one key can't collide), and to answer with the `next` observe call.
    let caller_session_id = req.caller_session_id.clone();

    let idem_key = match req.idempotency_key.as_deref() {
        Some(key) if key.len() > MAX_IDEM_KEY_LEN => {
            return Err(WorkflowError::InvalidDef(format!(
                "idempotency_key too long: {} bytes (max {})",
                key.len(),
                MAX_IDEM_KEY_LEN
            )));
        }
        Some(key) => Some(scoped_idem_key(key, caller_session_id.as_deref())),
        None => None,
    };

    // Idempotency short-circuit (unlocked fast path; re-checked under the lock).
    if let Some(key) = &idem_key {
        if let Some(run_id) = idem_hit(deps, key).await? {
            return Ok(start_response(run_id, caller_session_id.as_deref()));
        }
    }

    // Bound sub-workflow nesting: a node that opted into `workflow::start` could
    // otherwise recurse (sub-workflow → node → sub-workflow → …) without limit.
    let depth = caller_workflow_depth(deps, caller_session_id.as_deref()).await?;
    if depth > MAX_WORKFLOW_DEPTH {
        return Err(WorkflowError::InvalidDef(format!(
            "sub-workflow nesting depth {depth} exceeds the cap of {MAX_WORKFLOW_DEPTH}"
        )));
    }

    // The model step. A written id with no provider on this engine would fail its
    // node at runtime, so it is refused; the catalog is skipped when unreadable. An
    // inherited model runs the caller right now, so it is not checked.
    let registered = fetch_registered_models(deps).await;
    if registered.is_none() {
        tracing::warn!("router::models::list unavailable; skipping model validation");
    }
    let caller = if req.definition.nodes.values().any(inherits_model) {
        read_caller_model(deps, caller_session_id.as_deref()).await
    } else {
        CallerModel::NoSession
    };
    model_step(&mut req.definition, registered.as_ref(), &caller)
        .map_err(WorkflowError::InvalidDef)?;

    let _idem_guard = match &idem_key {
        Some(_) => Some(deps.locks.guard(IDEM_LOCK).await),
        None => None,
    };
    if let Some(key) = &idem_key {
        if let Some(run_id) = idem_hit(deps, key).await? {
            return Ok(start_response(run_id, caller_session_id.as_deref()));
        }
    }

    let run_id = new_run_id();
    let _guard = deps.locks.guard(&run_id).await;

    if let Some(key) = &idem_key {
        state::put_idem(&deps.iii, key, &run_id).await?;
    }
    state::put_def(&deps.iii, &run_id, &req.definition).await?;

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
        caller_session_id,
        caller_fs_root: req.caller_fs_root,
        idem_key,
        created_at: now,
        updated_at: now,
    };

    state::put_run(&deps.iii, &record).await?;

    crate::telemetry::record_run_started();

    // The Running record is already persisted; if the first tick fails to enqueue,
    // only the cron sweep would recover it (up to a sweep interval later). Mark the
    // run Failed best-effort so workflow::status / list don't surface a phantom
    // Running run in the meantime, and free the key so a retry starts a new run.
    if let Err(e) = enqueue_tick(&deps.iii, &run_id, 0).await {
        record.status = RunStatus::Failed;
        record.result_error = Some(format!("failed to enqueue initial tick: {e}"));
        record.updated_at = deps.now_ms();
        let _ = state::put_run(&deps.iii, &record).await;
        if let Some(key) = &record.idem_key {
            let _ = state::delete_idem(&deps.iii, key).await;
        }
        return Err(e);
    }

    Ok(start_response(run_id, record.caller_session_id.as_deref()))
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

    /// `well_formed_def` as a flat request: `read` fans out over `plan`'s docs and
    /// also reads `plan`; `output` defaults to `summarize`, the only unread node.
    fn flat_request() -> Value {
        json!({
            "nodes": {
                "plan": {"task": "Plan.", "model": "claude-3-5-haiku-20241022",
                         "schema": {"type": "object"}},
                "read": {"task": "Read.", "model": "claude-3-5-haiku-20241022",
                         "fanout": "plan.result.docs", "reads": "plan",
                         "schema": {"type": "object"}},
                "summarize": {"task": "Summarize.", "model": "claude-3-5-haiku-20241022",
                              "reads": "read"}
            }
        })
    }

    /// `flat_request` with `edit` applied, parsed.
    fn flat(edit: impl FnOnce(&mut Value)) -> Result<StartRequest, String> {
        let mut v = flat_request();
        edit(&mut v);
        serde_json::from_value::<StartRequest>(v).map_err(|e| e.to_string())
    }

    fn refusal(request: Value) -> String {
        serde_json::from_value::<StartRequest>(request)
            .expect_err("request should be refused")
            .to_string()
    }
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
                output,
                ..Default::default()
            },
            input: InputSpec {
                from: input_from.into(),
                template: None,
            },
            fanout: fanout_over.map(|over| FanoutSpec {
                over: over.to_string(),
                items: None,
            }),
            ..Default::default()
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
            max_concurrency: None,
        }
    }

    #[test]
    fn add_read_dependencies_folds_reads_into_depends_on() {
        // well_formed_def: `read` reads node:plan (BOTH input.from and fanout.over)
        // with depends_on=[]; `summarize` reads node:read with depends_on=[].
        let mut def = well_formed_def();
        add_read_dependencies(&mut def);
        // Read-refs become scheduling edges; the duplicate plan ref dedups to one.
        assert_eq!(def.nodes["read"].depends_on, vec!["plan".to_string()]);
        assert_eq!(def.nodes["summarize"].depends_on, vec!["read".to_string()]);
        // `plan` reads run input (no node ref) → nothing added.
        assert!(def.nodes["plan"].depends_on.is_empty());
        // Idempotent: a second pass changes nothing.
        add_read_dependencies(&mut def);
        assert_eq!(def.nodes["read"].depends_on, vec!["plan".to_string()]);
        // The folded def is exactly the well-formed shape, so it still validates.
        assert!(validate_def(&def).is_ok());
    }

    #[test]
    fn accepts_profile_without_model() {
        let req = serde_json::from_value::<StartRequest>(json!({
            "nodes": { "n": { "task": "t", "profile": "deepwork-lead" } }
        }))
        .expect("a profile node may omit the model");
        let agent = &req.definition.nodes["n"].agent;
        assert_eq!(agent.profile.as_deref(), Some("deepwork-lead"));
        assert!(agent.model.is_empty());
        assert!(validate_def(&req.definition).is_ok());
        // No model to check against the catalog.
        assert!(unknown_models(&req.definition, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn rejects_profile_with_system_prompt() {
        let mut def = well_formed_def();
        let agent = &mut def.nodes.get_mut("plan").unwrap().agent;
        agent.profile = Some("deepwork-lead".to_string());
        agent.system_prompt = Some("be terse".to_string());
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(
            err.contains(
                "node 'plan': profile supplies the system prompt; drop system_prompt or profile"
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_malformed_profile_id() {
        let mut def = well_formed_def();
        for bad in ["Deepwork-Lead", "a b", &"a".repeat(65), " ", ""] {
            def.nodes.get_mut("plan").unwrap().agent.profile = Some(bad.to_string());
            let err = validate_def(&def).unwrap_err().to_string();
            assert!(
                err.contains(&format!(
                    "node 'plan': profile {bad:?} is not a profile id \
                     (lowercase letters, digits, '-' and '_', at most 64 characters)"
                )),
                "{err}"
            );
        }
        for good in ["deepwork-lead", "smoke_echo2", &"a".repeat(64)] {
            def.nodes.get_mut("plan").unwrap().agent.profile = Some(good.to_string());
            assert!(validate_def(&def).is_ok(), "{good}");
        }
    }

    #[test]
    fn rejects_output_referencing_unknown_node() {
        // C1: an output.from pointing at a non-existent node would hang the run.
        let mut def = well_formed_def();
        def.output.from = "node:ghost".into();
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
            msg.contains("node 'plan' is read by a fanout but has no `schema`"),
            "error should name 'plan' and the fanout, got: {}",
            msg
        );
    }

    /// `well_formed_def` with `plan` turned into a text node read only by `input.from`:
    /// `read` keeps reading it whole but no longer fans out over it.
    fn text_plan_def() -> WorkflowDef {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.output = None;
        def.nodes.get_mut("read").unwrap().fanout = None;
        add_read_dependencies(&mut def);
        def
    }

    #[test]
    fn accepts_input_from_text_node() {
        let mut def = text_plan_def();
        assert!(validate_def(&def).is_ok(), "{:?}", validate_def(&def));
        def.nodes.get_mut("plan").unwrap().agent.output = Some(json!({"type": "text"}));
        assert!(validate_def(&def).is_ok(), "{:?}", validate_def(&def));
    }

    #[test]
    fn accepts_input_join_of_text_nodes() {
        let mut def = text_plan_def();
        def.nodes.get_mut("read").unwrap().agent.output = None;
        def.nodes.get_mut("summarize").unwrap().input.from =
            InputFrom::Many(vec!["node:plan".to_string(), "node:read".to_string()]);
        add_read_dependencies(&mut def);
        assert!(validate_def(&def).is_ok(), "{:?}", validate_def(&def));
    }

    #[test]
    fn rejects_input_from_unknown_node() {
        let mut def = well_formed_def();
        def.nodes.get_mut("summarize").unwrap().input.from = "node:ghost".into();
        add_read_dependencies(&mut def);
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("depends on unknown node ghost"), "{err}");
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
        let req = flat(|v| v["notify"] = json!({ "function_id": "myworker::wf-done" }))
            .expect("StartRequest with notify");
        let notify = req.notify.expect("notify present");
        assert_eq!(notify.function_id, "myworker::wf-done");
        assert!(
            notify.queue.is_none(),
            "queue defaults to None (→ \"default\")"
        );
    }

    #[test]
    fn start_refuses_reply_to_with_the_observe_recipe() {
        let mut with_reply_to = flat_request();
        with_reply_to["reply_to"] = json!({});
        for request in [
            with_reply_to,
            json!({ "reply_to": { "template": "Pick the winner:" } }),
        ] {
            let err = refusal(request);
            assert!(err.contains("`reply_to` was removed"), "{err}");
            assert!(err.contains("`next`") && err.contains("notify"), "{err}");
        }
        let req = flat(|_| {}).expect("StartRequest");
        assert!(req.caller_session_id.is_none());
    }

    #[test]
    fn next_is_the_observe_call_for_a_stamped_caller_only() {
        assert!(start_response("r_1".into(), None).next.is_none());
        let wire = serde_json::to_value(start_response("r_1".into(), None)).unwrap();
        assert_eq!(wire, json!({ "run_id": "r_1" }), "no `next` key");

        let next = start_response("r_1".into(), Some("s1"))
            .next
            .expect("next for an agent caller");
        assert_eq!(next["call"], "harness::observe");
        assert_eq!(
            next["arguments"],
            json!({
                "function_id": "workflow::on-run-completed",
                "args": { "run_id": "r_1" },
                "label": "workflow r_1",
                "lifecycle": { "expires_in_ms": 82_800_000 },
            })
        );
        assert_eq!(
            NEXT_EXPIRES_IN_MS,
            super::super::sweep::TERMINAL_RETENTION_MS - 3_600_000
        );
        let then = next["then"].as_str().unwrap();
        assert!(then.starts_with("END YOUR TURN"), "{then}");
        assert!(
            then.contains("`[notification] workflow r_1: {…}`"),
            "{then}"
        );
        assert!(
            then.contains("If the wake expires unfired, make this call again as given"),
            "{then}"
        );
    }

    #[test]
    fn start_request_parses_caller_session_id() {
        // The pre_call hook stamps `caller_session_id` into the args (from the
        // caller's turn) so node sessions can nest under the orchestrator.
        let req = flat(|v| {
            v["caller_session_id"] = json!("console-abc");
            v["caller_fs_root"] = json!("/home/u/proj");
        })
        .expect("StartRequest with caller_session_id");
        assert_eq!(req.caller_session_id.as_deref(), Some("console-abc"));
        assert_eq!(req.caller_fs_root.as_deref(), Some("/home/u/proj"));
    }

    /// Deps whose client never connects: enough for rules that run before any bus call.
    fn offline_deps() -> Deps {
        Deps {
            iii: std::sync::Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:9")),
            cfg: Default::default(),
            locks: Default::default(),
            events: std::sync::Arc::new(crate::events::test_support::RecordingDeliverer::new(
                false,
            )),
        }
    }

    #[tokio::test]
    async fn rejects_empty_caller_fs_root() {
        let req = flat(|v| v["caller_fs_root"] = json!(""))
            .expect("StartRequest with an empty caller_fs_root");
        let err = handle(&offline_deps(), req).await.unwrap_err().to_string();
        assert!(
            err.contains(
                "caller_fs_root must not be empty; omit it to use the harness default folder"
            ),
            "{err}"
        );
    }

    /// `well_formed_def` with `plan` rooted at `folder`.
    fn folder_def(folder: &str) -> WorkflowDef {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().agent.filesystem_root = Some(folder.to_string());
        def
    }

    fn folder_err(folder: &str, caller_fs_root: Option<&str>) -> String {
        validate_folders(&folder_def(folder), caller_fs_root)
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn filesystem_root_is_a_node_key() {
        let req = serde_json::from_value::<StartRequest>(json!({
            "nodes": { "a": { "task": "t", "model": "m", "filesystem_root": "/repo/wt" } }
        }))
        .expect("filesystem_root parses");
        assert_eq!(
            req.definition.nodes["a"].agent.filesystem_root.as_deref(),
            Some("/repo/wt")
        );
    }

    #[test]
    fn node_filesystem_root_must_be_absolute_without_dotdot() {
        let caller = tempfile::tempdir().unwrap();
        let caller = caller.path().to_str().unwrap();
        for bad in ["wt", "", "/repo/../etc", "/repo/wt/.."] {
            let err = folder_err(bad, Some(caller));
            assert!(
                err.contains(&format!(
                    "node 'plan': filesystem_root must be an absolute path without '..', \
                     got {bad:?}"
                )),
                "{bad}: {err}"
            );
        }
    }

    #[tokio::test]
    async fn node_filesystem_root_needs_caller_fs_root() {
        let req = flat(|v| v["nodes"]["plan"]["filesystem_root"] = json!("/repo/wt"))
            .expect("StartRequest with a node folder");
        let err = handle(&offline_deps(), req).await.unwrap_err().to_string();
        assert!(
            err.contains(
                "node 'plan': filesystem_root needs the caller's folder to confine it, \
                 and this run has none"
            ),
            "{err}"
        );
    }

    #[test]
    fn node_filesystem_root_must_lie_inside_caller_fs_root() {
        let base = tempfile::tempdir().unwrap();
        let repo = base.path().join("repo");
        std::fs::create_dir_all(repo.join("wt")).unwrap();
        std::fs::create_dir(base.path().join("repo2")).unwrap();
        let caller = repo.to_str().unwrap();
        let at = |rel: &str| base.path().join(rel).to_str().unwrap().to_string();

        // Inside (existing or created later by the run) and equal.
        for good in [at("repo/wt"), at("repo/.worktrees/kan-12"), at("repo")] {
            assert!(
                validate_folders(&folder_def(&good), Some(caller)).is_ok(),
                "{good}"
            );
        }
        // A sibling sharing the prefix, and a folder elsewhere.
        for bad in [at("repo2"), at("repo2/wt"), "/etc".to_string()] {
            let err = folder_err(&bad, Some(caller));
            assert!(
                err.contains(&format!(
                    "node 'plan': filesystem_root {bad:?} is outside the caller's folder \
                     {caller:?}; a node folder must lie inside it"
                )),
                "{bad}: {err}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn node_filesystem_root_through_a_symlink_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let repo = base.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.join("evil")).unwrap();
        std::os::unix::fs::symlink(base.path().join("gone"), repo.join("dangling")).unwrap();
        let caller = repo.to_str().unwrap();

        for rel in ["evil", "evil/wt", "dangling", "dangling/wt"] {
            let folder = repo.join(rel).to_str().unwrap().to_string();
            let err = folder_err(&folder, Some(caller));
            assert!(
                err.contains("is outside the caller's folder"),
                "{rel}: {err}"
            );
        }
    }

    #[test]
    fn node_filesystem_root_needs_the_caller_folder_on_this_host() {
        let base = tempfile::tempdir().unwrap();
        let caller = base.path().join("missing");
        let caller = caller.to_str().unwrap();
        let err = folder_err(&format!("{caller}/wt"), Some(caller));
        assert!(
            err.contains(&format!(
                "node 'plan': filesystem_root cannot be checked: the caller's folder \
                 {caller:?} does not exist on the workflow worker's host"
            )),
            "{err}"
        );
    }

    #[test]
    fn start_request_notify_is_optional() {
        let req = flat(|_| {}).expect("StartRequest without notify");
        assert!(req.notify.is_none());
    }

    #[test]
    fn engine_metadata_keys_are_not_refused() {
        let plain = flat(|_| {}).expect("plain request");
        let req = flat(|v| v["_caller_worker_id"] = json!("harness")).expect("engine key ignored");
        assert_eq!(req.definition, plain.definition);
    }

    #[test]
    fn stray_await_flag_is_refused() {
        // The blocking `await` mechanism was removed; an unknown top-level key is
        // refused rather than ignored, so a stale caller learns the contract.
        let schema = serde_json::to_value(schemars::schema_for!(StartRequest)).unwrap();
        assert!(
            schema["properties"].get("await").is_none(),
            "`await` must not appear in the request schema: {schema:#}"
        );
        let err = flat(|v| v["await"] = json!(true)).unwrap_err();
        assert!(err.contains("unknown top-level field `await`"), "{err}");
        assert!(err.contains("1 problem:"), "{err}");
    }

    #[test]
    fn collects_all_structural_problems_at_once() {
        // One refusal lists every problem (no whack-a-mole).
        let err = refusal(json!({
            "nodes": {
                "gen": { "fanout": "x.items" },
                "crit": { "task": "t", "reads": "gen", "depends_on": ["gen"] }
            },
            "output": { "from": "node:crit" }
        }));
        assert!(err.contains("node `gen`: missing `task`"), "{err}");
        assert!(
            err.contains("node `gen`: `fanout` names `x`, which is not a node (nodes: crit, gen)"),
            "{err}"
        );
        assert!(err.contains("`depends_on` is not a node field"), "{err}");
        assert!(err.contains("not {\"from\":…}"), "{err}");
        assert!(err.starts_with("workflow::start: 4 problems:"), "{err}");
        assert!(err.contains("Node keys: task (required)"), "{err}");
    }

    #[test]
    fn flags_misplaced_fields_with_hints() {
        // A node field at the top level, a top-level field on a node, and a node
        // written inside another node.
        let err = refusal(json!({
            "reads": "a",
            "nodes": { "n": {
                "task": "t",
                "max_concurrency": 2,
                "m": { "task": "inner" }
            } }
        }));
        assert!(
            err.contains("`reads` is a node field: put it inside a node under `nodes`"),
            "{err}"
        );
        assert!(
            err.contains("node `n`: `max_concurrency` is a top-level field: put it beside `nodes`"),
            "{err}"
        );
        assert!(
            err.contains("node `n`: `m` looks like a node written inside `n`; close `n` first"),
            "{err}"
        );
    }

    #[test]
    fn rejects_timeout_999_and_retries_11() {
        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().timeout_ms = Some(999);
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(
            err.contains("node 'plan': timeout_ms must be at least 1000 (milliseconds)"),
            "{err}"
        );

        let mut def = well_formed_def();
        def.nodes.get_mut("plan").unwrap().timeout_ms = Some(1000);
        def.nodes.get_mut("plan").unwrap().retries = Some(11);
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(
            err.contains("node 'plan': retries must be at most 10"),
            "{err}"
        );

        def.nodes.get_mut("plan").unwrap().retries = Some(10);
        assert!(validate_def(&def).is_ok());
    }

    #[test]
    fn rejects_zero_concurrency() {
        let mut def = well_formed_def();
        def.max_concurrency = Some(0);
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("max_concurrency must be at least 1"), "{err}");
        def.max_concurrency = Some(1);
        assert!(validate_def(&def).is_ok());
    }

    #[test]
    fn accepts_node_options_keys() {
        let req = serde_json::from_value::<StartRequest>(json!({
            "nodes": { "n": { "task": "t", "model": "m", "timeout_ms": 120000, "retries": 0 } },
            "max_concurrency": 2
        }))
        .expect("node options parse");
        assert_eq!(req.definition.max_concurrency, Some(2));
        assert_eq!(req.definition.nodes["n"].timeout_ms, Some(120_000));
        assert_eq!(req.definition.nodes["n"].retries, Some(0));
        assert!(validate_def(&req.definition).is_ok());
    }

    #[test]
    fn label_and_phase_parse_and_are_length_checked() {
        let req = serde_json::from_value::<StartRequest>(json!({
            "nodes": { "n": { "task": "t", "label": "KAN-12 · fix", "phase": "code" } }
        }))
        .expect("label and phase parse");
        assert_eq!(
            req.definition.nodes["n"].label.as_deref(),
            Some("KAN-12 · fix")
        );
        assert!(validate_def(&req.definition).is_ok());

        let mut def = req.definition.clone();
        def.nodes.get_mut("n").unwrap().phase = Some(" ".into());
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(
            err.contains("node 'n': phase must be non-empty and at most 80 characters"),
            "{err}"
        );
        let mut def = req.definition;
        def.nodes.get_mut("n").unwrap().label = Some("é".repeat(81));
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("node 'n': label must be"), "{err}");
        def.nodes.get_mut("n").unwrap().label = Some("é".repeat(80));
        assert!(validate_def(&def).is_ok());
    }

    /// Three independent text nodes with `output` as given.
    fn text_nodes_request(output: Value) -> StartRequest {
        serde_json::from_value::<StartRequest>(json!({
            "nodes": { "a": { "task": "t" }, "b": { "task": "t" }, "c": { "task": "t" } },
            "output": output
        }))
        .expect("parse")
    }

    #[test]
    fn output_array_of_text_nodes_validates() {
        let req = text_nodes_request(json!(["a", "b", "c"]));
        assert_eq!(req.definition.output.from.node_ids(), vec!["a", "b", "c"]);
        assert_eq!(
            req.definition.output.from,
            OutputFrom::Many(vec!["node:a".into(), "node:b".into(), "node:c".into()])
        );
        assert!(validate_def(&req.definition).is_ok());
    }

    #[test]
    fn stored_output_rejects_run_input_dotted_path_and_unknown_node() {
        for (bad, id) in [
            ("run_input", "run_input"),
            ("node:a.result", "a.result"),
            ("node:ghost", "ghost"),
        ] {
            let mut def = text_nodes_request(json!("a")).definition;
            def.output.from = OutputFrom::Many(vec!["node:b".into(), bad.into()]);
            let err = validate_def(&def).unwrap_err().to_string();
            assert!(
                err.contains(&format!("output references unknown node '{id}'")),
                "{err}"
            );
        }
    }

    #[test]
    fn stored_output_rejects_duplicates_and_empty() {
        let mut def = text_nodes_request(json!("a")).definition;
        def.output.from = OutputFrom::Many(vec![]);
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("output must name at least one node"), "{err}");
        let err = validate_def(&text_nodes_request(json!(["a", "a"])).definition)
            .unwrap_err()
            .to_string();
        assert!(err.contains("output lists node 'a' twice"), "{err}");
    }

    #[test]
    fn rejects_unknown_on_error_value() {
        let parse = |on_error: &str| {
            serde_json::from_value::<StartRequest>(json!({
                "nodes": { "n": { "task": "t", "on_error": on_error } }
            }))
        };
        let req = parse("continue").expect("continue parses");
        assert_eq!(
            req.definition.nodes["n"].on_error,
            crate::types::OnError::Continue
        );
        let err = parse("skip").expect_err("skip is rejected").to_string();
        assert!(err.contains("unknown variant `skip`"), "{err}");
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
    fn json_output_schema_must_compile_and_null_is_none() {
        let mut def = well_formed_def();
        let plan = |def: &mut WorkflowDef, schema: Value| {
            def.nodes.get_mut("plan").unwrap().agent.output =
                Some(json!({"type": "json", "schema": schema}));
        };
        plan(&mut def, json!("not a schema"));
        let err = validate_def(&def).unwrap_err().to_string();
        assert!(err.contains("not a valid JSON Schema"), "{err}");
        plan(&mut def, Value::Null);
        assert!(validate_def(&def).is_ok(), "schema: null means no schema");
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

    // Prevents: a key whose run was deleted handing back a dead run_id forever.
    #[test]
    fn idem_hit_only_when_run_exists() {
        let run: WorkflowRunRecord = serde_json::from_value(json!({
            "run_id": "r_1", "step": 0, "status": "running", "def_ref": "r_1",
            "input": {}, "created_at": 0, "updated_at": 0
        }))
        .unwrap();
        assert!(matches!(idem_hit_decision(Ok(Some(run))), IdemHit::Hit));
        assert!(matches!(idem_hit_decision(Ok(None)), IdemHit::Dangling));
        assert!(matches!(
            idem_hit_decision(Err(WorkflowError::State("state::get timed out".into()))),
            IdemHit::Err(_)
        ));
    }

    #[test]
    fn idem_parse_error_is_dangling() {
        let parse = serde_json::from_value::<WorkflowRunRecord>(json!({"run_id": 1})).unwrap_err();
        assert!(matches!(
            idem_hit_decision(Err(WorkflowError::Serde(parse))),
            IdemHit::Dangling
        ));
    }

    // -----------------------------------------------------------------------
    // Rule 6: loops
    // -----------------------------------------------------------------------

    /// The README's loop example: `work` → `verify` (loop over `work`) → `report`.
    fn loop_json() -> Value {
        json!({
            "nodes": {
                "work": {
                    "agent": {"profile": "deepwork-impl"},
                    "input": {"from": "run_input", "template": "Implement KAN-12; fix every finding in loop_previous."},
                    "on_error": "continue"
                },
                "verify": {
                    "agent": {"profile": "deepwork-adversary", "output": {"type": "json", "schema": {
                        "type": "object",
                        "properties": {
                            "verdict": {"type": "string", "enum": ["pass", "reject"]},
                            "findings": {"type": "array", "items": {"type": "string"}}
                        },
                        "required": ["verdict", "findings"]}}},
                    "input": {"from": "node:work", "template": "Review the work."},
                    "loop": {"body": ["work"], "while": {"field": "verdict", "equals": "reject"},
                             "max_repeats": 2},
                    "on_error": "continue"
                },
                "report": {"agent": {"model": "m"}, "input": {"from": "node:verify"}}
            },
            "output": {"from": "node:report"}
        })
    }

    /// Parse a stored definition, fold reads, validate.
    fn check_loop(definition: Value) -> Result<(), String> {
        let mut def: WorkflowDef = serde_json::from_value(definition).map_err(|e| e.to_string())?;
        add_read_dependencies(&mut def);
        validate_def(&def).map_err(|e| e.to_string())
    }

    fn loop_err(edit: impl FnOnce(&mut Value)) -> String {
        let mut v = loop_json();
        edit(&mut v);
        check_loop(v).expect_err("definition should be rejected")
    }

    #[test]
    fn valid_loop_definition_passes() {
        assert_eq!(check_loop(loop_json()), Ok(()));
        let err = loop_err(|v| v["nodes"]["verify"]["loop"]["until"] = json!(1));
        assert!(err.contains("until"), "{err}");
        // In a request, a loop's inner shape error comes from the typed parse, with the hint.
        let err = refusal(json!({"nodes": {"v": {"task": "t", "loop": {
            "while": {"field": "verdict", "equals": "x"}, "max_repeats": 1, "until": 1}}}}));
        assert!(err.contains("until") && err.contains("Shape: "), "{err}");
    }

    #[test]
    fn rejects_loop_max_repeats_out_of_range() {
        for n in [0, 10] {
            let err = loop_err(|v| v["nodes"]["verify"]["loop"]["max_repeats"] = json!(n));
            assert!(
                err.contains("node 'verify': loop.max_repeats must be between 1 and 9"),
                "{err}"
            );
        }
        let mut v = loop_json();
        v["nodes"]["verify"]["loop"]["max_repeats"] = json!(9);
        assert_eq!(check_loop(v), Ok(()));
    }

    #[test]
    fn rejects_loop_field_with_empty_segment() {
        for field in ["", "a..b", ".verdict", "verdict."] {
            let err = loop_err(|v| v["nodes"]["verify"]["loop"]["while"]["field"] = json!(field));
            assert!(
                err.contains(&format!(
                    "node 'verify': loop.while.field must be a dotted path without empty segments; got \"{field}\""
                )),
                "{err}"
            );
        }
    }

    #[test]
    fn rejects_empty_loop_equals() {
        let err = loop_err(|v| v["nodes"]["verify"]["loop"]["while"]["equals"] = json!(""));
        assert!(
            err.contains("node 'verify': loop.while.equals must not be empty"),
            "{err}"
        );
    }

    #[test]
    fn rejects_loop_on_text_node() {
        let err = loop_err(|v| {
            v["nodes"]["verify"]["agent"]
                .as_object_mut()
                .unwrap()
                .remove("output");
        });
        assert!(
            err.contains(
                "node 'verify': loop.while reads this node's result, so the node needs a `schema`"
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_loop_field_missing_from_declared_schema() {
        let err =
            loop_err(|v| v["nodes"]["verify"]["loop"]["while"]["field"] = json!("review.verdict"));
        assert!(
            err.contains(
                "node 'verify': loop.while.field = \"review.verdict\" is not declared in the \
                 node's schema (missing segment 'review')"
            ),
            "{err}"
        );
        // A dynamic schema cannot be checked for declared fields, but `required` still is.
        let mut v = loop_json();
        v["nodes"]["verify"]["loop"]["while"]["field"] = json!("review.verdict");
        v["nodes"]["verify"]["agent"]["output"]["schema"]["additionalProperties"] = json!(true);
        let err = check_loop(v.clone()).expect_err("an unrequired field is refused");
        assert!(
            err.contains(
                "node 'verify': loop.while.field = \"review.verdict\" must be listed in the \
                 schema's `required` (segment 'review')"
            ),
            "{err}"
        );
        v["nodes"]["verify"]["agent"]["output"]["schema"]["required"] =
            json!(["verdict", "findings", "review"]);
        assert_eq!(check_loop(v), Ok(()));
    }

    #[test]
    fn rejects_loop_field_missing_from_required() {
        let check = |schema: Value| {
            let mut v = loop_json();
            v["nodes"]["verify"]["loop"]["while"]["equals"] = json!("fail");
            v["nodes"]["verify"]["agent"]["output"]["schema"] = schema;
            check_loop(v)
        };
        let enum_schema = json!({"type": "object",
            "properties": {"verdict": {"type": "string", "enum": ["pass", "fail"]}},
            "required": ["verdict"]});
        assert_eq!(check(enum_schema), Ok(()));
        assert_eq!(
            check(json!({"type": "object", "required": ["verdict"]})),
            Ok(())
        );
        let err = check(json!({"type": "object", "properties": {"verdict": {"type": "string"}}}))
            .expect_err("an unrequired field is refused");
        assert!(
            err.contains(
                "node 'verify': loop.while.field = \"verdict\" must be listed in the schema's \
                 `required`; a result without it ends the loop after one pass"
            ),
            "{err}"
        );
        // additionalProperties does not skip the lookup; a composition keyword does.
        let err = check(json!({"type": "object", "additionalProperties": true}))
            .expect_err("additionalProperties still needs `required`");
        assert!(
            err.contains("must be listed in the schema's `required`"),
            "{err}"
        );
        for k in ["anyOf", "oneOf", "allOf"] {
            let schema = json!({"type": "object", k: [{"required": ["verdict"]}]});
            assert_eq!(check(schema), Ok(()), "{k}");
        }
        let schema = json!({"type": "object", "$ref": "#/$defs/v",
            "$defs": {"v": {"required": ["verdict"]}}});
        assert_eq!(check(schema), Ok(()));
        // A nested field needs each segment listed by its enclosing object.
        let mut v = loop_json();
        v["nodes"]["verify"]["loop"]["while"]["field"] = json!("review.verdict");
        v["nodes"]["verify"]["agent"]["output"]["schema"] = json!({"type": "object",
            "properties": {"review": {"type": "object",
                "properties": {"verdict": {"type": "string"}}}},
            "required": ["review"]});
        let err = check_loop(v.clone()).expect_err("inner segment is not required");
        assert!(err.contains("(segment 'verdict')"), "{err}");
        v["nodes"]["verify"]["agent"]["output"]["schema"]["properties"]["review"]["required"] =
            json!(["verdict"]);
        assert_eq!(check_loop(v), Ok(()));
    }

    #[test]
    fn rejects_loop_equals_outside_declared_enum() {
        let err = loop_err(|v| v["nodes"]["verify"]["loop"]["while"]["equals"] = json!("fail"));
        assert!(
            err.contains(
                "node 'verify': loop.while.equals = \"fail\" is not one of the values the \
                 schema declares for 'verdict' (pass, reject)"
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_loop_body_unknown_self_or_duplicate() {
        let err = loop_err(|v| v["nodes"]["verify"]["loop"]["body"] = json!(["work", "ghost"]));
        assert!(
            err.contains("node 'verify': loop.body references unknown node 'ghost'"),
            "{err}"
        );
        for (body, bad) in [
            (json!(["verify"]), "verify"),
            (json!(["work", "work"]), "work"),
        ] {
            let err = loop_err(|v| v["nodes"]["verify"]["loop"]["body"] = body);
            assert!(
                err.contains(&format!(
                    "node 'verify': loop.body must list other nodes once each; got '{bad}'"
                )),
                "{err}"
            );
        }
    }

    #[test]
    fn rejects_node_in_two_loops() {
        let err = loop_err(|v| {
            let second = v["nodes"]["verify"].clone();
            v["nodes"]["verify2"] = second;
        });
        assert!(
            err.contains(
                "node 'work' belongs to the loops of 'verify' and 'verify2'; a node belongs to \
                 at most one loop"
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_fanout_on_loop_node() {
        let err = loop_err(|v| {
            v["nodes"]["plan"] = json!({
                "agent": {"model": "m", "output": {"type": "json", "schema": {
                    "type": "object", "properties": {"items": {"type": "array"}}}}},
                "input": {"from": "run_input"}
            });
            v["nodes"]["work"]["input"] = json!({"from": "fanout_item"});
            v["nodes"]["work"]["fanout"] = json!({"over": "node:plan.items"});
        });
        assert!(
            err.contains("node 'work': a node of the loop 'verify' must not declare fanout"),
            "{err}"
        );
        // The loop node itself is a member too.
        let err = loop_err(|v| {
            v["nodes"]["plan"] = json!({
                "agent": {"model": "m", "output": {"type": "json", "schema": {
                    "type": "object", "properties": {"items": {"type": "array"}}}}},
                "input": {"from": "run_input"}
            });
            v["nodes"]["verify"]["input"] = json!({"from": "fanout_item"});
            v["nodes"]["verify"]["fanout"] = json!({"over": "node:plan.items"});
        });
        assert!(
            err.contains("node 'verify': a node of the loop 'verify' must not declare fanout"),
            "{err}"
        );
    }

    #[test]
    fn rejects_loop_field_with_non_string_type() {
        for (ty, shown) in [
            (json!("boolean"), "boolean"),
            (json!(["integer", "null"]), "integer|null"),
        ] {
            let err = loop_err(|v| {
                v["nodes"]["verify"]["agent"]["output"]["schema"]["properties"]["verdict"] =
                    json!({"type": ty});
            });
            assert!(
                err.contains(&format!(
                    "node 'verify': loop.while.field = \"verdict\" is declared as {shown}; the \
                     loop compares strings only"
                )),
                "{err}"
            );
        }
        // A string union still passes.
        let mut v = loop_json();
        v["nodes"]["verify"]["agent"]["output"]["schema"]["properties"]["verdict"]["type"] =
            json!(["string", "null"]);
        assert_eq!(check_loop(v), Ok(()));
    }

    #[test]
    fn rejects_loop_body_that_does_not_feed_loop_node() {
        let err = loop_err(|v| {
            v["nodes"]["side"] = json!({"agent": {"model": "m"}, "input": {"from": "run_input"}});
            v["nodes"]["verify"]["loop"]["body"] = json!(["work", "side"]);
        });
        assert!(
            err.contains(
                "node 'verify': loop.body node 'side' does not feed 'verify'; 'verify' must read \
                 every body node, directly or through other body nodes"
            ),
            "{err}"
        );
        // Through another body node is enough.
        let mut v = loop_json();
        v["nodes"]["fix"] = json!({"agent": {"model": "m"}, "input": {"from": "node:work"}});
        v["nodes"]["verify"]["input"] = json!({"from": "node:fix"});
        v["nodes"]["verify"]["loop"]["body"] = json!(["work", "fix"]);
        assert_eq!(check_loop(v), Ok(()));
    }

    #[test]
    fn rejects_body_reading_its_loop_node() {
        let err = loop_err(|v| v["nodes"]["work"]["input"] = json!({"from": "node:verify"}));
        assert!(
            err.contains(
                "node 'work' reads 'verify', the node that carries its loop; the previous result \
                 arrives as `loop_previous`"
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_outside_read_of_loop_body_and_output_naming_it() {
        let err = loop_err(|v| {
            v["nodes"]["report"]["input"] = json!({"from": ["node:verify", "node:work"]})
        });
        assert!(
            err.contains(
                "node 'report' reads 'work', a loop.body node of 'verify'; outside the loop read \
                 'verify', which returns the last pass"
            ),
            "{err}"
        );
        let err = loop_err(|v| v["output"] = json!({"from": ["node:report", "node:work"]}));
        assert!(
            err.contains(
                "output references 'work', a loop.body node of 'verify'; name 'verify' instead"
            ),
            "{err}"
        );
    }

    #[test]
    fn rejects_fanout_over_loop_node() {
        let err = loop_err(|v| {
            v["nodes"]["report"]["input"] = json!({"from": "fanout_item"});
            v["nodes"]["report"]["fanout"] = json!({"over": "node:verify.findings"});
        });
        assert!(
            err.contains(
                "node 'report': fanout reads 'verify', which carries a loop; a fanout cannot \
                 iterate a loop's result"
            ),
            "{err}"
        );
    }

    // -----------------------------------------------------------------------
    // The flat request: lowering, defaults, the shape pass
    // -----------------------------------------------------------------------

    fn parse(request: Value) -> StartRequest {
        serde_json::from_value::<StartRequest>(request.clone())
            .unwrap_or_else(|e| panic!("{request}: {e}"))
    }

    #[test]
    fn lower_matches_the_shared_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/flat_start_cases.json"))
                .expect("fixture parses");
        for case in fixture["cases"].as_array().expect("cases") {
            let name = &case["name"];
            let req = parse(case["request"].clone());
            assert_eq!(
                serde_json::to_value(&req.definition).unwrap(),
                case["stored"],
                "{name}"
            );
            assert_eq!(
                req.input,
                case["request"].get("input").cloned().unwrap_or(Value::Null),
                "{name}"
            );
            assert_eq!(
                validate_def(&req.definition).map_err(|e| e.to_string()),
                Ok(()),
                "{name}"
            );
        }
    }

    #[test]
    fn default_output_is_every_unread_node() {
        let one = parse(json!({"nodes": {"a": {"task": "t"}, "b": {"task": "t", "reads": "a"}}}));
        assert_eq!(one.definition.output.from, OutputFrom::One("node:b".into()));

        let several = parse(json!({"nodes": {"a": {"task": "t"}, "b": {"task": "t"}}}));
        assert_eq!(
            several.definition.output.from,
            OutputFrom::Many(vec!["node:a".into(), "node:b".into()])
        );

        let explicit =
            parse(json!({"nodes": {"a": {"task": "t"}, "b": {"task": "t"}}, "output": "a"}));
        assert_eq!(
            explicit.definition.output.from,
            OutputFrom::One("node:a".into())
        );
    }

    #[test]
    fn read_cycle_without_unread_nodes_reaches_the_cycle_or_loop_rule() {
        let cycle = parse(json!({"nodes": {
            "a": {"task": "t", "reads": "b"},
            "b": {"task": "t", "reads": "a"}
        }}));
        let err = validate_def(&cycle.definition).unwrap_err().to_string();
        assert!(err.contains("cycle through node"), "{err}");
        assert!(!err.contains("output"), "{err}");

        let looped = parse(json!({"nodes": {
            "work": {"task": "t", "reads": "check"},
            "check": {"task": "t", "reads": "work",
                      "schema": {"type": "object", "properties": {"verdict": {"type": "string"}},
                                 "required": ["verdict"]},
                      "loop": {"body": ["work"], "while": {"field": "verdict", "equals": "fail"},
                               "max_repeats": 2}}
        }}));
        assert_eq!(
            looped.definition.output.from,
            OutputFrom::One("node:check".into()),
            "a loop body node is never the default output"
        );
        let err = validate_def(&looped.definition).unwrap_err().to_string();
        assert!(
            err.contains("node 'work' reads 'check', the node that carries its loop"),
            "{err}"
        );
        assert!(!err.contains("output"), "{err}");
    }

    #[test]
    fn fanout_forms_lower_to_the_stored_pair() {
        let req = parse(json!({
            "nodes": {
                "plan": {"task": "t", "schema": {"type": "object",
                         "properties": {"docs": {"type": "array"}}}},
                "spec": {"task": "t"},
                "copy": {"task": "t", "fanout": "plan.docs"},
                "review": {"task": "t", "fanout": "plan.docs", "reads": ["spec", "plan"]},
                "angle": {"task": "t", "fanout": ["security", "cost"], "reads": "spec"},
                "solo": {"task": "t", "fanout": [1, {"k": 2}]}
            },
            "output": ["copy", "review", "angle", "solo"]
        }));
        let n = &req.definition.nodes;
        assert!(n["copy"].input.from.is_literal("fanout_item"));
        assert_eq!(n["copy"].depends_on, vec!["plan".to_string()]);

        assert_eq!(
            n["review"].input.from,
            InputFrom::Many(vec!["node:spec".into(), "node:plan".into()])
        );
        assert_eq!(
            n["review"].depends_on,
            vec!["spec".to_string(), "plan".to_string()]
        );

        let angle = n["angle"].fanout.as_ref().unwrap();
        assert!(angle.over.is_empty());
        assert_eq!(angle.items, Some(vec![json!("security"), json!("cost")]));
        assert_eq!(n["angle"].input.from, InputFrom::One("node:spec".into()));
        assert_eq!(n["angle"].depends_on, vec!["spec".to_string()]);

        assert!(n["solo"].input.from.is_literal("fanout_item"));
        assert!(
            n["solo"].depends_on.is_empty(),
            "a written array adds no dependency"
        );
        assert_eq!(
            validate_def(&req.definition).map_err(|e| e.to_string()),
            Ok(())
        );
    }

    #[test]
    fn fanout_copy_may_read_a_loop_node_but_not_its_body() {
        let request = |reads: &str| {
            json!({"nodes": {
                "work": {"task": "t"},
                "check": {"task": "t", "reads": "work",
                          "schema": {"type": "object", "properties": {"verdict": {"type": "string"}},
                                     "required": ["verdict"]},
                          "loop": {"body": ["work"], "while": {"field": "verdict", "equals": "fail"},
                                   "max_repeats": 2}},
                "critics": {"task": "t", "fanout": ["a", "b"], "reads": reads}
            }, "output": "critics"})
        };
        assert_eq!(
            validate_def(&parse(request("check")).definition).map_err(|e| e.to_string()),
            Ok(())
        );
        let err = validate_def(&parse(request("work")).definition)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("node 'critics' reads 'work', a loop.body node of 'check'"),
            "{err}"
        );
    }

    #[test]
    fn old_shape_request_names_each_recipe_once() {
        let err = refusal(json!({"definition": {
            "version": 1,
            "nodes": {
                "a": {"agent": {"model": "m"}, "input": {"from": "run_input", "template": "x"}},
                "b": {"agent": {"model": "m", "output": {"type": "json"}},
                      "input": {"from": "node:a"}, "depends_on": ["a"]},
                "c": {"agent": {"model": "m"}, "input": {"from": "node:b"}, "depends_on": ["b"]}
            },
            "output": {"from": "node:c"}
        }}));
        assert!(err.starts_with("workflow::start: 6 problems:"), "{err}");
        for (recipe, nodes) in [
            ("`definition` was removed", ""),
            ("`version` was removed: drop it", ""),
            ("not {\"from\":…}", ""),
            ("`agent` was removed: put `model`", "(node `a` and 2 more)"),
            ("a node has no `input`", "(node `a` and 2 more)"),
            ("`depends_on` is not a node field", "(node `b` and 1 more)"),
        ] {
            assert_eq!(err.matches(recipe).count(), 1, "{recipe}: {err}");
            let line = err.lines().find(|l| l.contains(recipe)).unwrap();
            assert!(line.ends_with(nodes), "{line}");
        }
        assert!(!err.contains("missing `task`"), "{err}");
    }

    #[test]
    fn shape_pass_rows() {
        let node = |extra: Value| {
            let mut a = json!({"task": "t"});
            a.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            json!({"nodes": {"a": a, "b": {"task": "t"}}})
        };
        let top = |key: &str, value: Value| {
            let mut v = json!({"nodes": {"a": {"task": "t"}, "b": {"task": "t"}}});
            v[key] = value;
            v
        };
        let cases: Vec<(Value, &str)> = vec![
            (top("reply_to", json!({})), "`reply_to` was removed"),
            (json!({"definition": {"nodes": {"a": {"task": "t"}}}}), "`definition` was removed"),
            (json!({"workflow": {"nodes": {"a": {"task": "t"}}}}), "`definition` was removed"),
            (top("version", json!(1)), "`version` was removed: drop it"),
            (
                top("c", json!({"task": "t"})),
                "`c` looks like a node written beside `nodes`: move it inside `nodes`",
            ),
            (top("model", json!("m")), "`model` is a node field: put it inside a node under `nodes`"),
            (top("name", json!("x")), "unknown top-level field `name`"),
            (json!({}), "missing `nodes`"),
            (json!({"nodes": {}}), "`nodes` is empty"),
            (json!({"nodes": [1]}), "`nodes` must be an object keyed by node id, not an array"),
            (json!({"nodes": {"a.b": {"task": "t"}}}), "node id `a.b` must not contain `#`, `.` or `/`"),
            (
                json!({"nodes": {"a": {"task": "t"}, "output": "a"}}),
                "`output` is a top-level field written inside `nodes`: close `nodes` first",
            ),
            (top("output", json!({"from": "node:a"})), "not {\"from\":…}"),
            (top("output", json!([])), "`output` must name a node; omit it to return every node"),
            (top("output", json!("node:a")), "`output`: write the bare id `a`, not `node:a`"),
            (top("output", json!("x")), "`output` names `x`, which is not a node (nodes: a, b)"),
            (
                top("max_concurrency", json!("2")),
                "`max_concurrency` must be a non-negative integer, not a string",
            ),
            (
                top("max_concurrency", json!(-1)),
                "`max_concurrency` must be a non-negative integer, not -1",
            ),
            (
                node(json!({"timeout_ms": 1.5})),
                "node `a`: `timeout_ms` must be a non-negative integer, not 1.5",
            ),
            (node(json!({"reads": []})), "node `a`: `reads` is an empty array"),
            (node(json!({"agent": {"model": "m"}})), "`agent` was removed: put `model`"),
            (node(json!({"agent": "p"})), "`agent` was removed: a profile id goes in \"profile\":\"x\" (node `a`)"),
            (node(json!({"input": {"from": "run_input"}})), "a node has no `input`"),
            (node(json!({"depends_on": ["b"]})), "`depends_on` is not a node field"),
            (node(json!({"output": {"type": "json"}})), "`output` is not a node field"),
            (
                node(json!({"notify": {"function_id": "f"}})),
                "node `a`: `notify` is a top-level field: put it beside `nodes`",
            ),
            (node(json!({"default_functions": []})), "node `a`: on a node the field is `functions`"),
            (
                node(json!({"c": {"task": "t"}})),
                "node `a`: `c` looks like a node written inside `a`; close `a` first",
            ),
            (node(json!({"x": 1})), "node `a`: unknown field `x`"),
            (json!({"nodes": {"a": {}}}), "node `a`: missing `task`, the text that tells this node"),
            (json!({"nodes": {"a": {"task": " "}}}), "node `a`: missing `task`"),
            (json!({"nodes": {"a": {"task": 1}}}), "node `a`: missing `task`"),
            (
                node(json!({"reads": {"from": "b"}})),
                "node `a`: `reads` must be a node id or an array of ids, not an object",
            ),
            (
                node(json!({"reads": ["b", 1]})),
                "node `a`: `reads` must be a node id or an array of ids, not an array holding a number",
            ),
            (node(json!({"reads": "node:b"})), "node `a`: write the bare id `b`, not `node:b`"),
            (node(json!({"fanout": "node:b.x"})), "node `a`: write the bare id `b.x`, not `node:b.x`"),
            (
                node(json!({"reads": "b.result"})),
                "node `a`: `reads` takes a whole result; only `fanout` takes a path. Write `b`",
            ),
            (
                node(json!({"reads": "x"})),
                "node `a`: `reads` names `x`, which is not a node (nodes: a, b). Omit `reads`",
            ),
            (node(json!({"reads": "run_input"})), "node `a`: the run `input` is not a node"),
            (node(json!({"fanout": "input.items"})), "node `a`: the run `input` is not a node"),
            (node(json!({"fanout": {"over": "node:b.x"}})), "not {\"over\":…}"),
            (node(json!({"fanout": []})), "node `a`: `fanout` is an empty array"),
            (
                node(json!({"fanout": "[\"x\"]"})),
                "node `a`: `fanout` holds an array written as a string",
            ),
            (node(json!({"fanout": "x.items"})), "node `a`: `fanout` names `x`, which is not a node"),
            (
                node(json!({"schema": {"type": "json", "schema": {"type": "object"}}})),
                "node `a`: `schema` is the JSON Schema itself",
            ),
            (node(json!({"schema": {"type": "text"}})), "node `a`: omit `schema` for a text node"),
            (node(json!({"provider": "groq"})), "node `a`: `provider` needs `model`"),
            (
                node(json!({"model": " ", "provider": "groq"})),
                "node `a`: `provider` needs `model`",
            ),
        ];
        for (request, expected) in cases {
            let err = refusal(request.clone());
            assert!(err.contains(expected), "{request}\n{expected}\n{err}");
            assert!(err.ends_with(SHAPE_HINT), "{err}");
        }
    }

    #[test]
    fn listed_node_ids_are_capped() {
        let mut nodes = serde_json::Map::new();
        for i in 0..25 {
            nodes.insert(format!("n{i:02}"), json!({"task": "t"}));
        }
        nodes.insert("r".into(), json!({"task": "t", "reads": "x"}));
        let err = refusal(json!({ "nodes": nodes }));
        assert!(err.contains("n19 and 6 more). Omit"), "{err}");
    }

    #[test]
    fn known_keys_pass_the_shape_pass() {
        // A run input may itself look like a node.
        parse(json!({"input": {"task": "Fix the login bug"}, "nodes": {"a": {"task": "t"}}}));
        // A profile node may set a provider; a node may read `input` when it is a node id.
        parse(json!({"nodes": {
            "input": {"task": "t"},
            "a": {"task": "t", "reads": "input", "profile": "p", "provider": "groq"}
        }}));
    }

    #[test]
    fn null_means_omitted() {
        let req = parse(json!({
            "nodes": {"a": {
                "task": "t", "reads": null, "fanout": ["x", "y"], "schema": null, "model": null,
                "provider": null, "profile": null, "system_prompt": null, "functions": null,
                "filesystem_root": null, "loop": null, "label": null, "phase": null,
                "timeout_ms": null, "retries": null, "on_error": null, "agent": null,
                "input": null, "depends_on": null
            }},
            "input": null, "output": null, "max_concurrency": null, "default_functions": null,
            "idempotency_key": null, "notify": null, "caller_session_id": null,
            "caller_fs_root": null, "definition": null, "version": null
        }));
        let a = &req.definition.nodes["a"];
        assert!(
            a.input.from.is_literal("fanout_item"),
            "the copy receives its bare element"
        );
        assert_eq!(a.on_error, crate::types::OnError::Fail);
        assert_eq!(a.agent, AgentSpec::default());
        assert_eq!(req.definition.output.from, OutputFrom::One("node:a".into()));
        assert_eq!(req.input, Value::Null);
    }

    #[test]
    fn start_schema_is_flat_open_and_self_documenting() {
        let schema = schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<StartRequest>();
        let schema = serde_json::to_value(schema).unwrap();
        assert_eq!(schema["title"], "StartRequest");
        assert_eq!(schema["required"], json!(["nodes"]));
        assert!(schema.get("additionalProperties").is_none(), "{schema:#}");
        for gone in ["definition", "workflow", "version"] {
            assert!(schema["properties"].get(gone).is_none(), "{gone}");
        }
        let node = &schema["definitions"]["Node"];
        assert_eq!(node["required"], json!(["task"]));
        assert!(node.get("additionalProperties").is_none(), "{node:#}");
        for gone in ["agent", "input", "depends_on", "output"] {
            assert!(node["properties"].get(gone).is_none(), "{gone}");
        }
        assert!(
            node["properties"]["schema"]["type"]
                .to_string()
                .contains("object"),
            "{node:#}"
        );
        let fanout = serde_json::to_string(&schema["definitions"]["Fanout"]).unwrap();
        assert!(
            fanout.contains("\"string\"") && fanout.contains("\"array\""),
            "{fanout}"
        );
        let blob = schema.to_string().to_lowercase();
        for needle in [
            "your own model",
            "router::models::list",
            "no other node reads",
            "runs per element",
            "fanout_item",
            "loop_previous",
            "own_previous",
            "grants no permissions",
        ] {
            assert!(blob.contains(needle), "the start schema lost {needle:?}");
        }
    }

    // -----------------------------------------------------------------------
    // The model step
    // -----------------------------------------------------------------------

    fn model_def() -> WorkflowDef {
        parse(json!({"nodes": {
            "plain": {"task": "t"},
            "prof": {"task": "t", "profile": "p"},
            "prof_pin": {"task": "t", "profile": "p", "provider": "groq"},
            "explicit": {"task": "t", "model": "m1"}
        }}))
        .definition
    }

    #[test]
    fn inherit_caller_model_fills_omitted_models() {
        let mut def = model_def();
        let caller = CallerModel::Known {
            model: "caller-m".into(),
            provider: Some("anthropic".into()),
        };
        assert!(inherit_caller_model(&mut def, &caller).is_empty());
        let agent = |id: &str| def.nodes[id].agent.clone();
        assert_eq!(agent("plain").model, "caller-m");
        assert_eq!(agent("plain").provider.as_deref(), Some("anthropic"));
        assert_eq!(agent("prof").model, "caller-m");
        assert_eq!(
            agent("prof").provider,
            None,
            "a profile node keeps the routed provider"
        );
        assert_eq!(
            agent("prof_pin").model,
            "",
            "a written provider is left as written"
        );
        assert_eq!(agent("prof_pin").provider.as_deref(), Some("groq"));
        assert_eq!(agent("explicit").model, "m1");

        for caller in [CallerModel::NoSession, CallerModel::Unreadable] {
            let mut def = model_def();
            assert_eq!(
                inherit_caller_model(&mut def, &caller),
                vec!["plain".to_string()]
            );
            assert_eq!(
                def.nodes["prof"].agent.model, "",
                "the profile's pin applies"
            );
        }
    }

    #[test]
    fn blank_model_and_provider_inherit_the_callers_model() {
        let req = parse(json!({"nodes": {"a": {"task": "t", "model": " ", "provider": ""}}}));
        assert!(inherits_model(&req.definition.nodes["a"]));
    }

    #[test]
    fn model_step_checks_written_ids_not_inherited_ones() {
        let registered = BTreeSet::from([("anthropic".to_string(), "m1".to_string())]);
        let caller = CallerModel::Known {
            model: "caller-only".into(),
            provider: None,
        };
        let mut def = model_def();
        assert_eq!(model_step(&mut def, Some(&registered), &caller), Ok(()));
        assert_eq!(def.nodes["plain"].agent.model, "caller-only");

        let mut def = model_def();
        def.nodes.get_mut("explicit").unwrap().agent.model = "nope".into();
        let err = model_step(&mut def, Some(&registered), &caller).unwrap_err();
        assert!(err.contains("unknown model(s) [\"nope\"]"), "{err}");
        assert!(!err.contains("caller-only"), "{err}");
    }

    #[test]
    fn caller_model_reads_the_status_context() {
        assert_eq!(
            caller_model_from_status(&json!({"context": {"model": "gpt", "provider": "openai"}})),
            CallerModel::Known {
                model: "gpt".into(),
                provider: Some("openai".into())
            }
        );
        assert_eq!(
            caller_model_from_status(&json!({"context": {"model": "gpt"}})),
            CallerModel::Known {
                model: "gpt".into(),
                provider: None
            }
        );
        for resp in [
            json!({}),
            json!({"context": null}),
            json!({"context": {"model": " "}}),
        ] {
            assert_eq!(
                caller_model_from_status(&resp),
                CallerModel::Unreadable,
                "{resp}"
            );
        }
    }

    #[test]
    fn model_problems_name_every_problem() {
        let registered = BTreeSet::from([("anthropic".to_string(), "m1".to_string())]);
        assert_eq!(
            model_problems(&[], &[], &CallerModel::NoSession, Some(&registered)),
            None
        );
        let msg = model_problems(
            &["gpt-4o-mini".to_string()],
            &["a".to_string(), "b".to_string()],
            &CallerModel::NoSession,
            Some(&registered),
        )
        .unwrap();
        assert!(
            msg.contains(
                "unknown model(s) [\"gpt-4o-mini\"]: not registered on this engine. Omit `model` \
                 to run on your own model, or use one of: m1"
            ),
            "{msg}"
        );
        assert!(
            msg.contains(
                "node(s) `a`, `b`: no `model`, and this call has no caller session to inherit one \
                 from; set `model` to one of: m1"
            ),
            "{msg}"
        );
        let msg = model_problems(&[], &["a".to_string()], &CallerModel::Unreadable, None).unwrap();
        assert_eq!(
            msg,
            "node(s) `a`: no `model`, and your session's model could not be read; set `model` to \
             an id from router::models::list"
        );
    }
}
