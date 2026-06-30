use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::Value;

use crate::ids::node_uid;
use crate::types::{NodeCheckpoint, NodeDef, NodeState, RunStatus, WorkflowDef, WorkflowRunRecord};

// ponytail: generous cap so a runaway `over` array can't materialize unbounded
// per-item state/sessions; tighten if abused.
const MAX_FANOUT_ITEMS: usize = 10_000;

// Per-RUN ceiling on total materialized node sessions. MAX_FANOUT_ITEMS bounds a
// SINGLE fanout, but multiple/chained fanouts multiply (10k × N could reach 100M),
// and the `over` array is LLM-controlled. This caps the product so one run can't
// fan out into a resource bomb (each item is a harness session + a checkpoint
// stored in the run's single JSON record). Safety ceiling, not a tuning knob.
const MAX_TOTAL_NODES: usize = 50_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns a zeroed-out Pending checkpoint (caller sets node_uid as the map key).
fn pending_checkpoint() -> NodeCheckpoint {
    NodeCheckpoint {
        state: NodeState::Pending,
        session_id: None,
        turn_id: None,
        result_ref: None,
        result_error: None,
        pending_at: None,
        pending_timeout_ms: None,
        retries: 0,
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns the materialized uids for a fanned-out node, e.g. `["read#0","read#1"]`,
/// in NUMERIC (not lexical) order. Returns an empty vec if the node has not yet been
/// expanded (i.e. `record.fanout_src` has no entry for `node_id`).
pub fn fanned_uids(record: &WorkflowRunRecord, node_id: &str) -> Vec<String> {
    match record.fanout_src.get(node_id) {
        None => vec![],
        Some(items) => (0..items.len())
            .map(|i| node_uid(node_id, Some(i as u32)))
            .collect(),
    }
}

/// JSON value kind, for diagnostics.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Resolve an `over` path like `"node:plan.result.docs"` against a prefetched results map
/// (node_uid → that node's result Value).
///
/// Protocol:
/// 1. Strip the `"node:"` prefix; everything that follows is a dotted path.
/// 2. The first segment is the node_id — look it up in `results`.
/// 3. Skip a literal `"result"` segment (the next one after the node_id).
/// 4. Walk the remaining segments into the JSON tree.
/// 5. Return the value only if it is an array (`Value::Array`).
///
/// On failure returns `Err(<reason>)` that NAMES the actual problem (missing node,
/// missing key with the keys that ARE present, a non-object on the way down, or a
/// non-array leaf) — never a misleading size-cap phrasing.
pub fn resolve_over_path(
    over: &str,
    results: &BTreeMap<String, Value>,
) -> Result<Vec<Value>, String> {
    let path = over
        .strip_prefix("node:")
        .ok_or_else(|| "path must start with \"node:\"".to_string())?;

    let parts = path.splitn(2, '.').collect::<Vec<_>>();
    let node_id = parts[0];

    let root = results
        .get(node_id)
        .ok_or_else(|| format!("node '{node_id}' has no result in scope"))?
        .clone();

    let rest = if parts.len() > 1 { parts[1] } else { "" };
    let segments: Vec<&str> = rest.split('.').filter(|s| !s.is_empty()).collect();

    // Skip the literal "result" segment (first of the remaining segments).
    let walk_segments: &[&str] = if segments.first() == Some(&"result") {
        &segments[1..]
    } else {
        &segments[..]
    };

    let mut cur = root;
    for seg in walk_segments {
        match cur {
            Value::Object(mut map) => {
                cur = map.remove(*seg).ok_or_else(|| {
                    let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
                    keys.sort();
                    format!(
                        "path segment '{seg}' missing; node '{node_id}' produced keys: {{{}}}",
                        keys.join(", ")
                    )
                })?;
            }
            other => {
                return Err(format!(
                    "path segment '{seg}' cannot be resolved: parent value is {} (not an object)",
                    json_type_name(&other)
                ));
            }
        }
    }

    match cur {
        Value::Array(arr) => Ok(arr),
        other => Err(format!(
            "value at path is {} (not an array)",
            json_type_name(&other)
        )),
    }
}

/// For each fanout node in `def` whose dependencies are Done and which has not yet been
/// expanded (no entry in `record.fanout_src`), snapshot the `over` array into
/// `record.fanout_src[node_id]` and insert a `Pending` `NodeCheckpoint` for each
/// `node_uid(node_id, Some(i))`.
///
/// Returns the list of expanded `node_id`s (not uids).  Caller is responsible for
/// persisting the record once.
pub fn expand_ready_fanouts(
    def: &WorkflowDef,
    record: &mut WorkflowRunRecord,
    results: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut expanded = Vec::new();

    // Collect candidates first to avoid borrow-checker issues on `record`.
    let candidates: Vec<(&str, &NodeDef)> = def
        .nodes
        .iter()
        .filter_map(|(id, node_def)| {
            if node_def.fanout.is_some() && !record.fanout_src.contains_key(id.as_str()) {
                Some((id.as_str(), node_def))
            } else {
                None
            }
        })
        .collect();

    for (node_id, node_def) in candidates {
        // All direct depends_on must be Done before we can snapshot the over array.
        if !deps_done(def, record, node_id) {
            continue;
        }

        // Resolve the over path. The source is Done; if `over` doesn't resolve
        // to an array (missing path / wrong shape) or is oversized, the fanout
        // can never expand — fail it fast (base-id Failed checkpoint) so the run
        // fails instead of parking in AwaitingNodes forever.
        let fanout_spec = node_def.fanout.as_ref().unwrap();
        let items = match resolve_over_path(&fanout_spec.over, results) {
            Ok(v) if v.len() <= MAX_FANOUT_ITEMS => v,
            outcome => {
                // Two distinct failures, each with an honest message:
                // - the path didn't resolve to an array (structural: names what was there)
                // - the array is genuinely oversize (the cap, stated as a cap)
                let reason = match outcome {
                    Ok(v) => format!(
                        "fanout '{}' over '{}': array has {} items, exceeds the cap of {}",
                        node_id,
                        fanout_spec.over,
                        v.len(),
                        MAX_FANOUT_ITEMS
                    ),
                    Err(detail) => {
                        format!(
                            "fanout '{}' over '{}': {}",
                            node_id, fanout_spec.over, detail
                        )
                    }
                };
                record.fanout_src.insert(node_id.to_string(), vec![]);
                let mut cp = pending_checkpoint();
                cp.state = NodeState::Failed;
                cp.result_error = Some(reason);
                record.nodes.insert(node_id.to_string(), cp);
                expanded.push(node_id.to_string());
                continue;
            }
        };

        // Run-wide cap: the per-node MAX_FANOUT_ITEMS bounds ONE fanout, but
        // chained/multiple fanouts multiply. Fail the fanout (base-id Failed, same
        // as the oversize path) if expanding it would push the run past the total
        // materialized-node ceiling, so an LLM-controlled `over` can't blow up the
        // record / spawn a session storm.
        if record.nodes.len() + items.len() > MAX_TOTAL_NODES {
            record.fanout_src.insert(node_id.to_string(), vec![]);
            let mut cp = pending_checkpoint();
            cp.state = NodeState::Failed;
            cp.result_error = Some(format!(
                "fanout '{}' over '{}': expanding {} items would exceed the run-wide cap of {} \
                 materialized nodes",
                node_id,
                fanout_spec.over,
                items.len(),
                MAX_TOTAL_NODES
            ));
            record.nodes.insert(node_id.to_string(), cp);
            expanded.push(node_id.to_string());
            continue;
        }

        // Atomically snapshot + insert Pending checkpoints.
        let n = items.len();
        record.fanout_src.insert(node_id.to_string(), items);
        for i in 0..n {
            let uid = node_uid(node_id, Some(i as u32));
            record.nodes.entry(uid).or_insert_with(pending_checkpoint);
        }

        expanded.push(node_id.to_string());
    }

    expanded
}

/// Returns `true` when every dependency of `node_id` is fully Done:
/// - For a normal (non-fanout) dependency: its checkpoint must be `NodeState::Done`.
/// - For a fanned-out dependency `dep_id`: the expansion must have occurred (entry in
///   `fanout_src`); an empty expansion is vacuously satisfied (0 items → 0..0 loop);
///   a non-empty expansion requires every `dep_id#i` checkpoint to be `NodeState::Done`.
///   An *un-expanded* fanout (no `fanout_src` entry) still blocks, as does a fanout that
///   FAILED to expand (a base-id `Failed`/`Cancelled` checkpoint).
pub fn deps_done(def: &WorkflowDef, record: &WorkflowRunRecord, node_id: &str) -> bool {
    let node_def = match def.nodes.get(node_id) {
        Some(n) => n,
        None => return false,
    };

    for dep_id in &node_def.depends_on {
        if def
            .nodes
            .get(dep_id)
            .and_then(|n| n.fanout.as_ref())
            .is_some()
        {
            // A base-id Failed/Cancelled checkpoint means the fanout FAILED to expand
            // (over path unresolvable / oversized / cap): `fanout_src` is an empty vec,
            // which the 0..0 loop below would otherwise read as vacuously Done. A failed
            // expansion must BLOCK its dependents (else an orphan-branch dependent fires
            // on Null input), not satisfy them. quiescence catches this for required
            // nodes; this covers orphan branches and the internal expand-time caller.
            if matches!(
                record.nodes.get(dep_id.as_str()).map(|c| c.state),
                Some(NodeState::Failed) | Some(NodeState::Cancelled)
            ) {
                return false;
            }
            // Fanned dependency: expansion must exist AND every #i must be Done.
            match record.fanout_src.get(dep_id.as_str()) {
                None => return false, // not yet expanded
                Some(items) => {
                    // Expanded-empty fanout (zero items) is vacuously Done — the 0..0 loop
                    // below satisfies the dependency. Only an *un-expanded* fanout (the `None`
                    // arm above) or a failed expansion (guard above) still blocks.
                    for i in 0..items.len() {
                        let uid = node_uid(dep_id, Some(i as u32));
                        match record.nodes.get(&uid) {
                            Some(cp) if cp.state == NodeState::Done => {}
                            _ => return false,
                        }
                    }
                }
            }
        } else {
            // Normal dependency: checkpoint must be Done.
            match record.nodes.get(dep_id.as_str()) {
                Some(cp) if cp.state == NodeState::Done => {}
                _ => return false,
            }
        }
    }

    true
}

/// Returns node uids that are not yet started and have all dependencies Done.
///
/// - For a fanout node that has been expanded: each materialized `#i` uid that is still
///   `Pending` (i.e. exists in `record.nodes` with state `Pending`).
/// - For a fanout node not yet expanded: skip (expansion happens first via
///   `expand_ready_fanouts`).
/// - For a normal node: the `node_id` itself if no checkpoint exists yet (never started).
pub fn ready_frontier(def: &WorkflowDef, record: &WorkflowRunRecord) -> Vec<String> {
    let mut frontier = Vec::new();

    for (node_id, node_def) in &def.nodes {
        if node_def.fanout.is_some() {
            // Fanout node: only emit already-materialized Pending items.
            if let Some(items) = record.fanout_src.get(node_id.as_str()) {
                for i in 0..items.len() {
                    let uid = node_uid(node_id, Some(i as u32));
                    if let Some(cp) = record.nodes.get(&uid) {
                        if cp.state == NodeState::Pending && deps_done(def, record, node_id) {
                            frontier.push(uid);
                        }
                    }
                }
            }
        } else {
            // Normal node: ready if no checkpoint yet and deps are done.
            if !record.nodes.contains_key(node_id.as_str()) && deps_done(def, record, node_id) {
                frontier.push(node_id.clone());
            }
        }
    }

    frontier
}

/// Build the input value for `node_id` by gathering results from its declared source.
///
/// - `input.from == "node:<dep>"` and `<dep>` is a FANOUT node → return a JSON array
///   `[results["<dep>#0"], results["<dep>#1"], …]` in strict NUMERIC order.
/// - `input.from == "node:<dep>"` and `<dep>` is a normal node → return that single
///   node's result, or `Value::Null` if not yet present.
/// - Any other `from` value (`run_input`, `literal`, `fanout_item`) → return
///   `record.input.clone()` (template/dispatch layer handles substitution later).
pub fn gather_input(
    def: &WorkflowDef,
    record: &WorkflowRunRecord,
    node_id: &str,
    results: &BTreeMap<String, Value>,
) -> Value {
    let node_def = match def.nodes.get(node_id) {
        Some(n) => n,
        None => return record.input.clone(),
    };

    match &node_def.input.from {
        crate::types::InputFrom::One(from) => gather_one(def, record, from, results),
        crate::types::InputFrom::Many(sources) => {
            // Join: gather each `node:<id>` into a field keyed by the dep id, so a
            // synthesis node can read every dependency it declared (a single
            // `from` could only deliver one — the silent-data-loss footgun).
            let mut obj = serde_json::Map::new();
            for src in sources {
                if let Some(dep) = src.strip_prefix("node:") {
                    let key = dep.split('.').next().unwrap_or(dep).to_string();
                    obj.insert(key, gather_one(def, record, src, results));
                }
                // Non-`node:` entries are rejected at workflow::start; ignore here.
            }
            Value::Object(obj)
        }
    }
}

/// Resolve a single `from` source against the results map (the per-source logic
/// shared by `One` and each element of `Many`).
fn gather_one(
    def: &WorkflowDef,
    record: &WorkflowRunRecord,
    from: &str,
    results: &BTreeMap<String, Value>,
) -> Value {
    if let Some(dep) = from.strip_prefix("node:") {
        // Check if dep is a fanout node.
        if def.nodes.get(dep).and_then(|n| n.fanout.as_ref()).is_some() {
            // Fan-in: iterate 0..N in NUMERIC order (not BTreeMap/lexical order).
            let n = fanned_uids(record, dep).len();
            let arr: Vec<Value> = (0..n)
                .map(|i| {
                    let uid = node_uid(dep, Some(i as u32));
                    results.get(&uid).cloned().unwrap_or(Value::Null)
                })
                .collect();
            Value::Array(arr)
        } else {
            // Normal node: return its single result.
            results.get(dep).cloned().unwrap_or(Value::Null)
        }
    } else {
        // run_input / literal / fanout_item → delegate to the template layer.
        record.input.clone()
    }
}

/// Compute the current quiescence state of a workflow run.
///
/// Evaluation order (first matching rule wins):
/// 1. `record.abort` → `Cancelled`
/// 2. Any node *in the required set* (or a fanned item of one) is `Failed`/`Cancelled` → `Failed`
/// 3. Every required node (and every `#i` of a required fanned node) is `Done` → `Completed`
/// 4. Otherwise → `AwaitingNodes`
///
/// Orphan-branch state (nodes not in the transitive closure of the output node) never
/// blocks or fails the run.
pub fn quiescence(def: &WorkflowDef, record: &WorkflowRunRecord) -> RunStatus {
    if record.abort {
        return RunStatus::Cancelled;
    }
    let required = required_set(def);

    let mut all_done = true;
    for node_id in &required {
        let is_fanout = def
            .nodes
            .get(node_id.as_str())
            .map(|n| n.fanout.is_some())
            .unwrap_or(false);
        // Expand a required node to the uids that actually carry state: a fanout
        // node contributes all its #i items, a normal node is itself.
        let uids = if is_fanout {
            fanned_uids(record, node_id.as_str())
        } else {
            vec![node_id.to_string()]
        };
        // A required fanout that hasn't expanded yet is not done. An expanded-empty fanout
        // (fanout_src entry present, zero items) is vacuously Done, so only an *un-expanded*
        // fanout blocks completion.
        if is_fanout {
            if !record.fanout_src.contains_key(node_id.as_str()) {
                all_done = false;
                continue;
            }
            // A base-id Failed/Cancelled checkpoint marks a fanout that failed to
            // expand (over path unresolvable / oversized).
            if matches!(
                record.nodes.get(node_id.as_str()).map(|c| c.state),
                Some(NodeState::Failed) | Some(NodeState::Cancelled)
            ) {
                return RunStatus::Failed;
            }
        }
        for uid in uids {
            match record.nodes.get(&uid).map(|c| c.state) {
                Some(NodeState::Failed) | Some(NodeState::Cancelled) => return RunStatus::Failed,
                Some(NodeState::Done) => {}
                _ => all_done = false, // Pending / Running / unstarted
            }
        }
    }
    if all_done {
        RunStatus::Completed
    } else {
        RunStatus::AwaitingNodes
    }
}

/// Reject a definition whose `depends_on` edges contain a cycle (would hang forever).
pub fn validate_acyclic(def: &WorkflowDef) -> Result<(), String> {
    // DFS with a recursion stack; colors: 0=unvisited,1=on-stack,2=done.
    let mut color: std::collections::HashMap<&str, u8> = std::collections::HashMap::new();
    fn visit<'a>(
        node: &'a str,
        def: &'a WorkflowDef,
        color: &mut std::collections::HashMap<&'a str, u8>,
    ) -> Result<(), String> {
        match color.get(node) {
            Some(2) => return Ok(()),
            Some(1) => return Err(format!("cycle through node {node}")),
            _ => {}
        }
        color.insert(node, 1);
        if let Some(n) = def.nodes.get(node) {
            for dep in &n.depends_on {
                if !def.nodes.contains_key(dep) {
                    return Err(format!("node {node} depends on unknown node {dep}"));
                }
                visit(dep, def, color)?;
            }
        }
        color.insert(node, 2);
        Ok(())
    }
    for node in def.nodes.keys() {
        visit(node, def, &mut color)?;
    }
    Ok(())
}

/// The output node plus the transitive closure of its `depends_on` (node_ids).
pub fn required_set(def: &WorkflowDef) -> BTreeSet<String> {
    let mut req = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let output = def
        .output
        .from
        .strip_prefix("node:")
        .unwrap_or(&def.output.from)
        .to_string();
    stack.push(output);
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        req.insert(n.clone());
        if let Some(node) = def.nodes.get(&n) {
            for dep in &node.depends_on {
                stack.push(dep.clone());
            }
        }
    }
    req
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    // -----------------------------------------------------------------------
    // Test helpers: the 3-node plan → read (fanout) → synthesize example
    // -----------------------------------------------------------------------

    fn def() -> WorkflowDef {
        use crate::types::{AgentSpec, FanoutSpec, InputSpec, NodeDef, OutputRef, WorkflowDef};
        let mut nodes = BTreeMap::new();

        nodes.insert(
            "plan".to_string(),
            NodeDef {
                agent: AgentSpec {
                    model: "claude-opus-4-8".to_string(),
                    provider: None,
                    system_prompt: None,
                    functions: None,
                    output: Some(json!({"type":"json"})),
                },
                input: InputSpec {
                    from: "run_input".into(),
                    template: Some("List the docs to read for: {{topic}}".to_string()),
                },
                depends_on: vec![],
                fanout: None,
            },
        );

        nodes.insert(
            "read".to_string(),
            NodeDef {
                agent: AgentSpec {
                    model: "claude-haiku-4-5".to_string(),
                    provider: None,
                    system_prompt: None,
                    functions: None,
                    output: Some(json!({"type":"json"})),
                },
                input: InputSpec {
                    from: "fanout_item".into(),
                    template: Some("Read and summarize: {{item}}".to_string()),
                },
                depends_on: vec!["plan".to_string()],
                fanout: Some(FanoutSpec {
                    over: "node:plan.result.docs".to_string(),
                }),
            },
        );

        nodes.insert(
            "synthesize".to_string(),
            NodeDef {
                agent: AgentSpec {
                    model: "claude-opus-4-8".to_string(),
                    provider: None,
                    system_prompt: None,
                    functions: None,
                    output: Some(json!({"type":"json"})),
                },
                input: InputSpec {
                    from: "node:read".into(),
                    template: Some("Synthesize from: {{results}}".to_string()),
                },
                depends_on: vec!["read".to_string()],
                fanout: None,
            },
        );

        WorkflowDef {
            version: 1,
            nodes,
            output: OutputRef {
                from: "node:synthesize".into(),
            },
            default_functions: None,
        }
    }

    fn record() -> WorkflowRunRecord {
        WorkflowRunRecord {
            run_id: "run_test".to_string(),
            step: 0,
            status: RunStatus::Running,
            abort: false,
            def_ref: "run_test".to_string(),
            input: json!({"topic": "rust"}),
            nodes: BTreeMap::new(),
            fanout_src: BTreeMap::new(),
            result: None,
            result_error: None,
            notify: None,
            reply_to: None,
            caller_session_id: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn done_checkpoint() -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Done,
            session_id: None,
            turn_id: None,
            result_ref: None,
            result_error: None,
            pending_at: None,
            pending_timeout_ms: None,
            retries: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Tests from the brief
    // -----------------------------------------------------------------------

    #[test]
    fn frontier_starts_with_root_node() {
        assert_eq!(ready_frontier(&def(), &record()), vec!["plan".to_string()]);
    }

    #[test]
    fn fanout_expands_from_frozen_source_in_numeric_order() {
        let (d, mut r) = (def(), record());
        r.nodes.insert("plan".into(), done_checkpoint());
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs":["a","b"]}));
        let expanded = expand_ready_fanouts(&d, &mut r, &results);
        assert_eq!(expanded, vec!["read".to_string()]);
        assert_eq!(r.fanout_src["read"], vec![json!("a"), json!("b")]);
        assert_eq!(
            fanned_uids(&r, "read"),
            vec!["read#0".to_string(), "read#1".to_string()]
        );
    }

    #[test]
    fn barrier_node_not_ready_until_all_fanned_items_done() {
        let (d, mut r) = (def(), record());

        // Expand read fanout with 2 items.
        r.nodes.insert("plan".into(), done_checkpoint());
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs":["a","b"]}));
        expand_ready_fanouts(&d, &mut r, &results);

        // Mark read#0 Done but NOT read#1.
        r.nodes.insert("read#0".into(), done_checkpoint());
        // read#1 remains Pending (set by expand_ready_fanouts).

        // synthesize depends on "read"; barrier must not be satisfied yet.
        assert!(
            !deps_done(&d, &r, "synthesize"),
            "synthesize should not be ready yet"
        );
        let frontier = ready_frontier(&d, &r);
        assert!(
            !frontier.contains(&"synthesize".to_string()),
            "synthesize must not appear in frontier until all read#i are Done"
        );

        // Now mark read#1 Done too.
        r.nodes.insert("read#1".into(), done_checkpoint());
        assert!(
            deps_done(&d, &r, "synthesize"),
            "synthesize should now be ready"
        );
        let frontier2 = ready_frontier(&d, &r);
        assert!(
            frontier2.contains(&"synthesize".to_string()),
            "synthesize must appear in frontier once all read#i are Done"
        );
    }

    // -----------------------------------------------------------------------
    // Additional correctness tests
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_over_path_returns_array() {
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs":["x","y","z"]}));
        let arr = resolve_over_path("node:plan.result.docs", &results);
        assert_eq!(arr, Ok(vec![json!("x"), json!("y"), json!("z")]));
    }

    #[test]
    fn resolve_over_path_errs_for_non_array_naming_the_type() {
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs": "not-an-array"}));
        let err = resolve_over_path("node:plan.result.docs", &results).unwrap_err();
        assert!(
            err.contains("not an array"),
            "should name the shape problem: {err}"
        );
        assert!(err.contains("string"), "should name the actual type: {err}");
    }

    #[test]
    fn resolve_over_path_errs_for_missing_key_listing_present_keys() {
        let mut results = BTreeMap::new();
        results.insert(
            "draft".to_string(),
            json!({"blog_post": "...", "title": "t"}),
        );
        let err = resolve_over_path("node:draft.rewrite_priority", &results).unwrap_err();
        assert!(
            err.contains("rewrite_priority"),
            "names the missing segment: {err}"
        );
        assert!(err.contains("produced keys"), "lists what WAS there: {err}");
        assert!(err.contains("blog_post"), "shows the real keys: {err}");
        assert!(
            !err.contains("10000"),
            "must NOT misleadingly mention a size cap: {err}"
        );
    }

    #[test]
    fn resolve_over_path_errs_for_missing_node() {
        let results: BTreeMap<String, Value> = BTreeMap::new();
        let err = resolve_over_path("node:plan.result.docs", &results).unwrap_err();
        assert!(
            err.contains("no result in scope"),
            "should name the missing node: {err}"
        );
    }

    #[test]
    fn fanned_uids_empty_when_not_expanded() {
        let r = record();
        assert_eq!(fanned_uids(&r, "read"), Vec::<String>::new());
    }

    #[test]
    fn expand_ready_fanouts_idempotent() {
        let (d, mut r) = (def(), record());
        r.nodes.insert("plan".into(), done_checkpoint());
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs":["a","b","c"]}));

        let first = expand_ready_fanouts(&d, &mut r, &results);
        let second = expand_ready_fanouts(&d, &mut r, &results);

        assert_eq!(first, vec!["read".to_string()]);
        assert!(second.is_empty(), "second call must be a no-op");
        // Snapshot must still have 3 items.
        assert_eq!(r.fanout_src["read"].len(), 3);
    }

    #[test]
    fn fanout_items_use_numeric_not_lexical_order() {
        let (d, mut r) = (def(), record());
        r.nodes.insert("plan".into(), done_checkpoint());
        // 11 items so that lexical order "read#10" < "read#2" would differ from numeric.
        let items: Vec<Value> = (0..11).map(|i| json!(i)).collect();
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs": items}));
        expand_ready_fanouts(&d, &mut r, &results);

        let uids = fanned_uids(&r, "read");
        assert_eq!(uids[0], "read#0");
        assert_eq!(uids[1], "read#1");
        assert_eq!(uids[9], "read#9");
        assert_eq!(uids[10], "read#10");
        // Numeric: read#10 must be last, not between read#1 and read#2.
        let pos2 = uids.iter().position(|u| u == "read#2").unwrap();
        let pos10 = uids.iter().position(|u| u == "read#10").unwrap();
        assert!(
            pos2 < pos10,
            "read#2 must come before read#10 in numeric order"
        );
    }

    #[test]
    fn frontier_does_not_include_fanout_node_id_before_expansion() {
        let (d, mut r) = (def(), record());
        // plan is done, but read has not been expanded yet.
        r.nodes.insert("plan".into(), done_checkpoint());
        let frontier = ready_frontier(&d, &r);
        // "read" itself must not appear; its items (read#0, read#1) also not yet.
        assert!(!frontier.contains(&"read".to_string()));
    }

    #[test]
    fn frontier_includes_pending_fanout_items_after_expansion() {
        let (d, mut r) = (def(), record());
        r.nodes.insert("plan".into(), done_checkpoint());
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs":["a","b"]}));
        expand_ready_fanouts(&d, &mut r, &results);

        let frontier = ready_frontier(&d, &r);
        assert!(frontier.contains(&"read#0".to_string()));
        assert!(frontier.contains(&"read#1".to_string()));
    }

    // -----------------------------------------------------------------------
    // Task 4 tests
    // -----------------------------------------------------------------------

    fn failed_checkpoint() -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Failed,
            session_id: None,
            turn_id: None,
            result_ref: None,
            result_error: None,
            pending_at: None,
            pending_timeout_ms: None,
            retries: 0,
        }
    }

    fn running_checkpoint() -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Running,
            session_id: None,
            turn_id: None,
            result_ref: None,
            result_error: None,
            pending_at: None,
            pending_timeout_ms: None,
            retries: 0,
        }
    }

    #[test]
    fn fan_in_gathers_results_in_numeric_order_not_lexical() {
        let (d, mut r) = (def(), record());

        // Expand read fanout with 11 items so lexical order (read#10 < read#2) would differ.
        r.nodes.insert("plan".into(), done_checkpoint());
        let items: Vec<Value> = (0..11).map(|i| json!(i)).collect();
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs": items}));
        expand_ready_fanouts(&d, &mut r, &results);

        // Mark all read#i Done and put their results in the map.
        for i in 0..11u32 {
            let uid = node_uid("read", Some(i));
            r.nodes.insert(uid.clone(), done_checkpoint());
            results.insert(uid, json!({"summary": i}));
        }

        // synthesize has input.from = "node:read" — gather_input should fan-in.
        let gathered = gather_input(&d, &r, "synthesize", &results);
        let arr = gathered.as_array().expect("expected a JSON array");
        assert_eq!(arr.len(), 11);
        // Element at index 2 must be the result of read#2.
        assert_eq!(arr[2], json!({"summary": 2}));
        // Element at index 10 must be the result of read#10 (not lexical last).
        assert_eq!(arr[10], json!({"summary": 10}));
        // Prove numeric vs lexical: index 2 comes before index 10 in the array.
        // (In lexical order "read#10" sorts before "read#2", so arr[2] would have been summary 10.)
        assert_ne!(
            arr[2],
            json!({"summary": 10}),
            "lexical order would have placed read#10 at position 2"
        );
    }

    #[test]
    fn many_input_gathers_each_dep_into_keyed_object() {
        use crate::types::{AgentSpec, InputSpec, NodeDef, OutputRef};
        let agent = || AgentSpec {
            model: "m".to_string(),
            provider: None,
            system_prompt: None,
            functions: None,
            output: Some(json!({"type":"json"})),
        };
        let mut nodes = BTreeMap::new();
        nodes.insert(
            "b".to_string(),
            NodeDef {
                agent: agent(),
                input: InputSpec {
                    from: "run_input".into(),
                    template: None,
                },
                depends_on: vec![],
                fanout: None,
            },
        );
        nodes.insert(
            "c".to_string(),
            NodeDef {
                agent: agent(),
                input: InputSpec {
                    from: "run_input".into(),
                    template: None,
                },
                depends_on: vec![],
                fanout: None,
            },
        );
        nodes.insert(
            "join".to_string(),
            NodeDef {
                agent: agent(),
                input: InputSpec {
                    from: InputFrom::Many(vec!["node:b".to_string(), "node:c".to_string()]),
                    template: None,
                },
                depends_on: vec!["b".to_string(), "c".to_string()],
                fanout: None,
            },
        );
        let d = WorkflowDef {
            version: 1,
            nodes,
            output: OutputRef {
                from: "node:join".into(),
            },
            default_functions: None,
        };
        let r = record();
        let mut results = BTreeMap::new();
        results.insert("b".to_string(), json!({"draft": "hello"}));
        results.insert("c".to_string(), json!({"review": "ok"}));

        // The join reads BOTH deps — each keyed by its node id (a single `from`
        // could only have delivered one of them).
        let gathered = gather_input(&d, &r, "join", &results);
        assert_eq!(
            gathered,
            json!({"b": {"draft": "hello"}, "c": {"review": "ok"}})
        );
    }

    #[test]
    fn quiescence_completed_only_when_output_done_and_nothing_running() {
        let (d, mut r) = (def(), record());

        // Set up: plan Done, read fanout expanded and all Done, synthesize Running.
        r.nodes.insert("plan".into(), done_checkpoint());
        let items: Vec<Value> = (0..2).map(|i| json!(i)).collect();
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs": items}));
        expand_ready_fanouts(&d, &mut r, &results);
        r.nodes.insert("read#0".into(), done_checkpoint());
        r.nodes.insert("read#1".into(), done_checkpoint());

        // Output node (synthesize) is Running → must be AwaitingNodes.
        r.nodes.insert("synthesize".into(), running_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::AwaitingNodes);

        // Flip synthesize to Done → must be Completed.
        r.nodes.insert("synthesize".into(), done_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);
    }

    #[test]
    fn quiescence_cancelled_on_abort_and_failed_on_node_failure() {
        let (d, mut r) = (def(), record());

        // abort=true → Cancelled, regardless of node states.
        r.abort = true;
        assert_eq!(quiescence(&d, &r), RunStatus::Cancelled);

        // abort=false but a node has failed → Failed.
        r.abort = false;
        r.nodes.insert("plan".into(), failed_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Failed);
    }

    #[test]
    fn validate_acyclic_accepts_a_diamond() {
        // a -> b, a -> c, b -> d, c -> d
        let d: WorkflowDef = serde_json::from_value(serde_json::json!({
            "version":1, "output":{"from":"node:d"}, "nodes":{
              "a":{"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"run_input","template":"t"}},
              "b":{"depends_on":["a"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:a","template":"t"}},
              "c":{"depends_on":["a"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:a","template":"t"}},
              "d":{"depends_on":["b","c"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:b","template":"t"}}
            }})).unwrap();
        assert!(validate_acyclic(&d).is_ok());
    }

    #[test]
    fn validate_acyclic_rejects_a_cycle() {
        // b -> c, c -> b
        let d: WorkflowDef = serde_json::from_value(serde_json::json!({
            "version":1, "output":{"from":"node:b"}, "nodes":{
              "b":{"depends_on":["c"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:c","template":"t"}},
              "c":{"depends_on":["b"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:b","template":"t"}}
            }})).unwrap();
        assert!(validate_acyclic(&d).is_err());
    }

    #[test]
    fn required_set_is_output_transitive_closure_only() {
        // a -> b -> d (required); a -> orphan (NOT required by output d)
        let d: WorkflowDef = serde_json::from_value(serde_json::json!({
            "version":1, "output":{"from":"node:d"}, "nodes":{
              "a":{"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"run_input","template":"t"}},
              "b":{"depends_on":["a"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:a","template":"t"}},
              "d":{"depends_on":["b"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:b","template":"t"}},
              "orphan":{"depends_on":["a"],"agent":{"model":"m","output":{"type":"json"}},"input":{"from":"node:a","template":"t"}}
            }})).unwrap();
        let req = required_set(&d);
        assert!(req.contains("a") && req.contains("b") && req.contains("d"));
        assert!(!req.contains("orphan"));
    }

    // -----------------------------------------------------------------------
    // Empty-fanout vacuous-completion tests (TDD — written before the fix)
    // -----------------------------------------------------------------------

    /// Minimal 3-node def: a (normal) → b (fanout over node:a.result.items) → c (normal).
    /// Output: node:c.
    fn empty_fanout_def() -> WorkflowDef {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "output": {"from": "node:c"},
            "nodes": {
                "a": {
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "run_input", "template": "t"},
                    "depends_on": []
                },
                "b": {
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "fanout_item", "template": "t"},
                    "depends_on": ["a"],
                    "fanout": {"over": "node:a.result.items"}
                },
                "c": {
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "node:b", "template": "t"},
                    "depends_on": ["b"]
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn deps_done_true_for_dependent_of_empty_fanout() {
        let d = empty_fanout_def();
        let mut r = record();
        // a is Done.
        r.nodes.insert("a".into(), done_checkpoint());
        // b was expanded to zero items — no b#i checkpoints inserted.
        r.fanout_src.insert("b".into(), vec![]);
        // c depends on b; b was expanded-empty → vacuously Done.
        assert!(
            deps_done(&d, &r, "c"),
            "deps_done must be true for c when b's fanout was expanded to empty"
        );
    }

    #[test]
    fn quiescence_completes_with_empty_required_fanout() {
        let d = empty_fanout_def();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.fanout_src.insert("b".into(), vec![]);
        r.nodes.insert("c".into(), done_checkpoint());
        assert_eq!(
            quiescence(&d, &r),
            RunStatus::Completed,
            "quiescence must be Completed when b expanded to empty and c is Done"
        );
    }

    #[test]
    fn quiescence_awaits_unexpanded_fanout() {
        let d = empty_fanout_def();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        // NO fanout_src entry for b — not yet expanded.
        assert_eq!(
            quiescence(&d, &r),
            RunStatus::AwaitingNodes,
            "quiescence must be AwaitingNodes when b has not been expanded yet"
        );
    }

    /// A required fanout whose source is Done but whose `over` doesn't resolve to
    /// an array must FAIL the run, not park it in AwaitingNodes forever.
    #[test]
    fn fanout_over_non_array_fails_run_instead_of_hanging() {
        let d = empty_fanout_def();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        // a's result has `items` as a non-array → `over` can't resolve to an array.
        let mut results = BTreeMap::new();
        results.insert("a".to_string(), json!({"items": "not-an-array"}));

        let expanded = expand_ready_fanouts(&d, &mut r, &results);
        assert_eq!(
            expanded,
            vec!["b".to_string()],
            "b is marked failed-to-expand"
        );
        assert_eq!(
            r.nodes.get("b").map(|c| c.state),
            Some(NodeState::Failed),
            "a base-id Failed checkpoint marks the failed fanout"
        );
        assert_eq!(
            quiescence(&d, &r),
            RunStatus::Failed,
            "run must fail (not hang) when a required fanout cannot expand"
        );
    }

    // -----------------------------------------------------------------------
    // Task 2: transitive multi-sink quiescence helpers and tests
    // -----------------------------------------------------------------------

    /// Returns the Task-1 required_set def: a -> b -> d (output), a -> orphan (not required).
    fn def_with_orphan() -> WorkflowDef {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "output": {"from": "node:d"},
            "nodes": {
                "a": {
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "run_input", "template": "t"}
                },
                "b": {
                    "depends_on": ["a"],
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "node:a", "template": "t"}
                },
                "d": {
                    "depends_on": ["b"],
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "node:b", "template": "t"}
                },
                "orphan": {
                    "depends_on": ["a"],
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "node:a", "template": "t"}
                }
            }
        }))
        .unwrap()
    }

    #[test]
    fn quiescence_ignores_orphan_branch_failure() {
        // required: a->b->d (all Done). orphan Failed. Run should be Completed, not Failed.
        let d = def_with_orphan();
        let mut r = record();
        for n in ["a", "b", "d"] {
            r.nodes.insert(n.into(), done_checkpoint());
        }
        r.nodes.insert("orphan".into(), failed_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);
    }

    #[test]
    fn quiescence_fails_on_required_branch_failure() {
        let d = def_with_orphan();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), failed_checkpoint()); // b is required by output d
        assert_eq!(quiescence(&d, &r), RunStatus::Failed);
    }

    #[test]
    fn quiescence_awaits_while_a_required_node_runs() {
        let d = def_with_orphan();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), running_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::AwaitingNodes);
    }
}
