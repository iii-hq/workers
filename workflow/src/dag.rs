use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::Value;

use crate::ids::{iter_uid, node_uid, split_iteration};
use crate::types::{
    LoopCondition, NodeCheckpoint, NodeDef, NodeState, OnError, OutputFrom, RunStatus, WorkflowDef,
    WorkflowRunRecord,
};

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

/// Node id → the loop node of the loop it belongs to (a loop node maps to itself).
/// Nodes in no loop are absent.
fn loop_groups(def: &WorkflowDef) -> BTreeMap<&str, &str> {
    let mut groups = BTreeMap::new();
    for (id, node) in &def.nodes {
        if let Some(spec) = &node.loop_spec {
            groups.insert(id.as_str(), id.as_str());
            for b in &spec.body {
                groups.insert(b.as_str(), id.as_str());
            }
        }
    }
    groups
}

/// The last materialized loop pass of `node_id`: probes `<id>#it1`, `<id>#it2`, …
/// until one is missing; 0 for a node that never repeated.
fn last_pass(record: &WorkflowRunRecord, node_id: &str) -> u32 {
    let mut k = 0;
    while record.nodes.contains_key(&iter_uid(node_id, k + 1)) {
        k += 1;
    }
    k
}

/// Uid of the last materialized loop pass of `node_id` (`node_id` itself for a node
/// that never repeated): what a reader outside the loop waits for and reads.
pub fn last_uid(record: &WorkflowRunRecord, node_id: &str) -> String {
    iter_uid(node_id, last_pass(record, node_id))
}

/// `cond.field` (a dotted path) of `result` is the string `cond.equals`. A missing
/// field, a non-string value or no result does not hold. Pure.
fn condition_holds(cond: &LoopCondition, result: Option<&Value>) -> bool {
    let leaf = result.and_then(|r| cond.field.split('.').try_fold(r, |cur, seg| cur.get(seg)));
    leaf.and_then(Value::as_str) == Some(cond.equals.as_str())
}

/// Start the next pass of every loop whose last pass `k` finished: when every node
/// of the loop is Done at pass `k`, `while` holds on the loop node's pass-`k` result
/// and `k < max_repeats`, insert a default Pending checkpoint `<node>#it<k+1>` for
/// every node of the loop, all or none. A failed or cancelled node (even a tolerated
/// one), a false condition, the repeat limit or the run-wide node cap ends the loop
/// quietly. A second call adds nothing. Returns the loop node ids that got a pass;
/// the caller persists the record. Call it before `expand_ready_fanouts` and
/// `ready_frontier`, so readers of a loop node see its new pass.
pub fn expand_ready_loops(
    def: &WorkflowDef,
    record: &mut WorkflowRunRecord,
    results: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut expanded = Vec::new();
    for (id, node) in &def.nodes {
        let Some(spec) = &node.loop_spec else {
            continue;
        };
        let k = last_pass(record, id);
        let members: Vec<&str> = std::iter::once(id.as_str())
            .chain(spec.body.iter().map(String::as_str))
            .collect();
        let pass_done = members.iter().all(|m| {
            record
                .nodes
                .get(&iter_uid(m, k))
                .is_some_and(|cp| cp.state == NodeState::Done)
        });
        if !pass_done
            || k >= spec.max_repeats
            || !condition_holds(&spec.cond, results.get(&iter_uid(id, k)))
            || record.nodes.len() + members.len() > MAX_TOTAL_NODES
        {
            continue;
        }
        for m in members {
            record.nodes.entry(iter_uid(m, k + 1)).or_default();
        }
        expanded.push(id.clone());
    }
    expanded
}

/// What a node of a loop reads at the pass of `uid`: `loop_previous`, the loop
/// node's result of the previous pass, and for a body node `Some(own_previous)`,
/// its own result of the previous pass (`None` for the loop node, where it would
/// equal `loop_previous`). Both are `null` on the first pass (or when that pass
/// left no result). `None` for a node in no loop.
pub fn loop_previous(
    def: &WorkflowDef,
    uid: &str,
    results: &BTreeMap<String, Value>,
) -> Option<(Value, Option<Value>)> {
    let (node_id, k) = split_iteration(uid);
    let loop_id = *loop_groups(def).get(node_id)?;
    let prev = |id: &str| match k {
        0 => Value::Null,
        k => results
            .get(&iter_uid(id, k - 1))
            .cloned()
            .unwrap_or(Value::Null),
    };
    Some((prev(loop_id), (node_id != loop_id).then(|| prev(node_id))))
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
/// expanded (no entry in `record.fanout_src`), snapshot its `items`, else the `over`
/// array, into
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
        // All direct depends_on (the `over` source and the reads) must be settled first.
        if !deps_done(def, record, node_id) {
            continue;
        }

        // Resolve the over path. The source is Done; if `over` doesn't resolve
        // to an array (missing path / wrong shape) or is oversized, the fanout
        // can never expand — fail it fast (base-id Failed checkpoint) so the run
        // fails instead of parking in AwaitingNodes forever.
        let fanout_spec = node_def.fanout.as_ref().unwrap();
        // The request spelling, so a runtime error does not teach the stored `node:` prefix.
        let source = match &fanout_spec.items {
            Some(_) => "its own array".to_string(),
            None => format!(
                "'{}'",
                fanout_spec
                    .over
                    .strip_prefix("node:")
                    .unwrap_or(&fanout_spec.over)
            ),
        };
        let outcome = match &fanout_spec.items {
            Some(items) => Ok(items.clone()),
            None => resolve_over_path(&fanout_spec.over, results),
        };
        let items = match outcome {
            Ok(v) if v.len() <= MAX_FANOUT_ITEMS => v,
            outcome => {
                // Two distinct failures, each with an honest message:
                // - the path didn't resolve to an array (structural: names what was there)
                // - the array is genuinely oversize (the cap, stated as a cap)
                let reason = match outcome {
                    Ok(v) => format!(
                        "fanout '{node_id}' over {source}: array has {} items, exceeds the cap of {MAX_FANOUT_ITEMS}",
                        v.len(),
                    ),
                    Err(detail) => format!("fanout '{node_id}' over {source}: {detail}"),
                };
                record.fanout_src.insert(node_id.to_string(), vec![]);
                let cp = NodeCheckpoint {
                    state: NodeState::Failed,
                    result_error: Some(reason),
                    ..Default::default()
                };
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
            let cp = NodeCheckpoint {
                state: NodeState::Failed,
                result_error: Some(format!(
                    "fanout '{node_id}' over {source}: expanding {} items would exceed the \
                     run-wide cap of {MAX_TOTAL_NODES} materialized nodes",
                    items.len(),
                )),
                ..Default::default()
            };
            record.nodes.insert(node_id.to_string(), cp);
            expanded.push(node_id.to_string());
            continue;
        }

        // Atomically snapshot + insert Pending checkpoints.
        let n = items.len();
        record.fanout_src.insert(node_id.to_string(), items);
        for i in 0..n {
            let uid = node_uid(node_id, Some(i as u32));
            record.nodes.entry(uid).or_default();
        }

        expanded.push(node_id.to_string());
    }

    expanded
}

/// A node in `state` is finished for its consumers: Done, or Failed/Cancelled when its
/// node is `on_error: continue` (`tolerant`). Pure.
pub fn is_settled(state: NodeState, tolerant: bool) -> bool {
    match state {
        NodeState::Done => true,
        NodeState::Failed | NodeState::Cancelled => tolerant,
        NodeState::Pending | NodeState::Running => false,
    }
}

/// True when `node_id` (a base id; fanout items share it) is `on_error: continue`.
pub(crate) fn tolerant(def: &WorkflowDef, node_id: &str) -> bool {
    def.nodes
        .get(node_id)
        .is_some_and(|n| n.on_error == OnError::Continue)
}

/// Returns `true` when every dependency of `uid` (a node id, a fanout item, or a loop
/// pass `<id>#it<k>`) is settled (`is_settled`):
/// - For a normal (non-fanout) dependency: its checkpoint must be settled; for a
///   dependency in the reader's loop, its checkpoint at the reader's pass; for any
///   other, its last pass (`last_uid`).
/// - For a fanned-out dependency `dep_id`: the expansion must have occurred (entry in
///   `fanout_src`); an empty expansion is vacuously satisfied (0 items → 0..0 loop);
///   a non-empty expansion requires every `dep_id#i` checkpoint to be settled.
///   An *un-expanded* fanout (no `fanout_src` entry) still blocks, as does a fanout that
///   FAILED to expand (a base-id `Failed`/`Cancelled` checkpoint) unless it is tolerant.
pub fn deps_done(def: &WorkflowDef, record: &WorkflowRunRecord, uid: &str) -> bool {
    deps_settled(def, record, uid, &loop_groups(def))
}

/// `deps_done` with the definition's `loop_groups` computed once by the caller.
fn deps_settled(
    def: &WorkflowDef,
    record: &WorkflowRunRecord,
    uid: &str,
    groups: &BTreeMap<&str, &str>,
) -> bool {
    let (_, pass) = split_iteration(uid);
    let node_id = uid.split('#').next().unwrap_or(uid);
    let node_def = match def.nodes.get(node_id) {
        Some(n) => n,
        None => return false,
    };
    let group = groups.get(node_id);

    for dep_id in &node_def.depends_on {
        let tolerant = tolerant(def, dep_id);
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
            // on Null input), not satisfy them — unless the fanout is tolerant, whose
            // consumers then read `[]`. quiescence catches this for required nodes; this
            // covers orphan branches and the internal expand-time caller.
            if !tolerant
                && matches!(
                    record.nodes.get(dep_id.as_str()).map(|c| c.state),
                    Some(NodeState::Failed) | Some(NodeState::Cancelled)
                )
            {
                return false;
            }
            // Fanned dependency: expansion must exist AND every #i must be settled.
            match record.fanout_src.get(dep_id.as_str()) {
                None => return false, // not yet expanded
                Some(items) => {
                    // Expanded-empty fanout (zero items) is vacuously Done — the 0..0 loop
                    // below satisfies the dependency. Only an *un-expanded* fanout (the `None`
                    // arm above) or a failed expansion (guard above) still blocks.
                    for i in 0..items.len() {
                        let uid = node_uid(dep_id, Some(i as u32));
                        match record.nodes.get(&uid) {
                            Some(cp) if is_settled(cp.state, tolerant) => {}
                            _ => return false,
                        }
                    }
                }
            }
        } else {
            // Same loop: the reader's own pass. Anything else: its last pass.
            let dep_uid = if group.is_some() && groups.get(dep_id.as_str()) == group {
                iter_uid(dep_id, pass)
            } else {
                last_uid(record, dep_id)
            };
            match record.nodes.get(&dep_uid) {
                Some(cp) if is_settled(cp.state, tolerant) => {}
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
/// - For a normal node: the `node_id` itself if it has no checkpoint yet or a
///   `Pending` one (reserved, a send to retry, or a sweep refire), and each loop pass
///   `<id>#it<k>` whose checkpoint is `Pending`.
pub fn ready_frontier(def: &WorkflowDef, record: &WorkflowRunRecord) -> Vec<String> {
    let mut frontier = Vec::new();
    let groups = loop_groups(def);

    for (node_id, node_def) in &def.nodes {
        if node_def.fanout.is_some() {
            // Fanout node: only emit already-materialized Pending items.
            if let Some(items) = record.fanout_src.get(node_id.as_str()) {
                for i in 0..items.len() {
                    let uid = node_uid(node_id, Some(i as u32));
                    if let Some(cp) = record.nodes.get(&uid) {
                        if cp.state == NodeState::Pending
                            && deps_settled(def, record, node_id, &groups)
                        {
                            frontier.push(uid);
                        }
                    }
                }
            }
        } else {
            // Normal node: each pass is ready if not started (no checkpoint or Pending)
            // and its deps are done.
            for k in 0..=last_pass(record, node_id) {
                let uid = iter_uid(node_id, k);
                let unstarted = record
                    .nodes
                    .get(&uid)
                    .is_none_or(|cp| cp.state == NodeState::Pending);
                if unstarted && deps_settled(def, record, &uid, &groups) {
                    frontier.push(uid);
                }
            }
        }
    }

    frontier
}

/// Build the input value for `uid` (a node id, or a loop pass `<id>#it<k>`) by
/// gathering results from its declared source. A `node:<dep>` in the reader's loop
/// reads the reader's pass; any other dependency reads its last pass.
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
    uid: &str,
    results: &BTreeMap<String, Value>,
) -> Value {
    let (_, pass) = split_iteration(uid);
    let node_id = uid.split('#').next().unwrap_or(uid);
    let node_def = match def.nodes.get(node_id) {
        Some(n) => n,
        None => return record.input.clone(),
    };
    let groups = loop_groups(def);
    let group = groups.get(node_id);
    let gather_one = |from: &str| match from.strip_prefix("node:") {
        // A node of the reader's loop: the reader's own pass.
        Some(dep) if group.is_some() && groups.get(dep) == group => results
            .get(&iter_uid(dep, pass))
            .cloned()
            .unwrap_or(Value::Null),
        Some(dep) => gather_node(def, record, dep, results),
        // run_input / literal / fanout_item → delegate to the template layer.
        None => record.input.clone(),
    };

    match &node_def.input.from {
        crate::types::InputFrom::One(from) => gather_one(from),
        crate::types::InputFrom::Many(sources) => {
            // Join: gather each `node:<id>` into a field keyed by the dep id, so a
            // synthesis node can read every dependency it declared (a single
            // `from` could only deliver one — the silent-data-loss footgun).
            let mut obj = serde_json::Map::new();
            for src in sources {
                if let Some(dep) = src.strip_prefix("node:") {
                    let key = dep.split('.').next().unwrap_or(dep).to_string();
                    obj.insert(key, gather_one(src));
                }
                // Non-`node:` entries are rejected at workflow::start; ignore here.
            }
            Value::Object(obj)
        }
    }
}

/// One node's result: a fanout group's children as an array in NUMERIC order, else
/// the node's own result (its last loop pass). A node without a result (unfinished, or failed with
/// `on_error: continue`) reads as `null`.
fn gather_node(
    def: &WorkflowDef,
    record: &WorkflowRunRecord,
    node_id: &str,
    results: &BTreeMap<String, Value>,
) -> Value {
    if def
        .nodes
        .get(node_id)
        .and_then(|n| n.fanout.as_ref())
        .is_some()
    {
        let n = fanned_uids(record, node_id).len();
        let arr: Vec<Value> = (0..n)
            .map(|i| {
                let uid = node_uid(node_id, Some(i as u32));
                results.get(&uid).cloned().unwrap_or(Value::Null)
            })
            .collect();
        Value::Array(arr)
    } else {
        results
            .get(&last_uid(record, node_id))
            .cloned()
            .unwrap_or(Value::Null)
    }
}

/// The run's result from `output.from`: one node's result as is, or an object keyed
/// by node id for the array form. Never the run input, even for a bare id.
pub fn gather_output(
    def: &WorkflowDef,
    record: &WorkflowRunRecord,
    results: &BTreeMap<String, Value>,
) -> Value {
    let ids = def.output.from.node_ids();
    match &def.output.from {
        OutputFrom::One(_) => gather_node(def, record, ids[0], results),
        OutputFrom::Many(_) => Value::Object(
            ids.into_iter()
                .map(|id| (id.to_string(), gather_node(def, record, id, results)))
                .collect(),
        ),
    }
}

/// Compute the current quiescence state of a workflow run.
///
/// Evaluation order (first matching rule wins):
/// 1. `record.abort` → `Cancelled`
/// 2. Any node *in the required set* (or a fanned item of one) is `Failed`/`Cancelled`
///    and not `on_error: continue` → `Failed`
/// 3. Every required node (every `#i` of a required fanned node, every pass of a
///    loop node) is settled (`is_settled`) → `Completed`
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
        let tolerant = tolerant(def, node_id);
        let is_fanout = def
            .nodes
            .get(node_id.as_str())
            .map(|n| n.fanout.is_some())
            .unwrap_or(false);
        // Expand a required node to the uids that actually carry state: a fanout
        // node contributes all its #i items, a normal node every materialized pass.
        let uids = if is_fanout {
            fanned_uids(record, node_id.as_str())
        } else {
            (0..=last_pass(record, node_id))
                .map(|k| iter_uid(node_id, k))
                .collect()
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
            if !tolerant
                && matches!(
                    record.nodes.get(node_id.as_str()).map(|c| c.state),
                    Some(NodeState::Failed) | Some(NodeState::Cancelled)
                )
            {
                return RunStatus::Failed;
            }
        }
        for uid in uids {
            match record.nodes.get(&uid).map(|c| c.state) {
                Some(s) if is_settled(s, tolerant) => {}
                Some(NodeState::Failed) | Some(NodeState::Cancelled) => return RunStatus::Failed,
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

/// The output node(s) plus the transitive closure of their `depends_on` (node_ids).
pub fn required_set(def: &WorkflowDef) -> BTreeSet<String> {
    let mut req = BTreeSet::new();
    let mut stack: Vec<String> = def
        .output
        .from
        .node_ids()
        .into_iter()
        .map(str::to_string)
        .collect();
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
                    output: Some(json!({"type":"json"})),
                    ..Default::default()
                },
                input: InputSpec {
                    from: "run_input".into(),
                    template: Some("List the docs to read for: {{topic}}".to_string()),
                },
                ..Default::default()
            },
        );

        nodes.insert(
            "read".to_string(),
            NodeDef {
                agent: AgentSpec {
                    model: "claude-haiku-4-5".to_string(),
                    output: Some(json!({"type":"json"})),
                    ..Default::default()
                },
                input: InputSpec {
                    from: "fanout_item".into(),
                    template: Some("Read and summarize: {{item}}".to_string()),
                },
                depends_on: vec!["plan".to_string()],
                fanout: Some(FanoutSpec {
                    over: "node:plan.result.docs".to_string(),
                    items: None,
                }),
                ..Default::default()
            },
        );

        nodes.insert(
            "synthesize".to_string(),
            NodeDef {
                agent: AgentSpec {
                    model: "claude-opus-4-8".to_string(),
                    output: Some(json!({"type":"json"})),
                    ..Default::default()
                },
                input: InputSpec {
                    from: "node:read".into(),
                    template: Some("Synthesize from: {{results}}".to_string()),
                },
                depends_on: vec!["read".to_string()],
                ..Default::default()
            },
        );

        WorkflowDef {
            version: 1,
            nodes,
            output: OutputRef {
                from: "node:synthesize".into(),
            },
            default_functions: None,
            max_concurrency: None,
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
            caller_session_id: None,
            caller_fs_root: None,
            idem_key: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn done_checkpoint() -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Done,
            ..Default::default()
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
            ..Default::default()
        }
    }

    fn running_checkpoint() -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Running,
            ..Default::default()
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
            output: Some(json!({"type":"json"})),
            ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
            },
        );
        let d = WorkflowDef {
            version: 1,
            nodes,
            output: OutputRef {
                from: "node:join".into(),
            },
            default_functions: None,
            max_concurrency: None,
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
        assert_eq!(
            r.nodes["b"].result_error.as_deref(),
            Some("fanout 'b' over 'a.result.items': value at path is string (not an array)")
        );
    }

    /// `draft` → `critic` (fanout over a written array, reading `draft`) → `final`.
    fn critics_def(reads: bool) -> WorkflowDef {
        let mut d: WorkflowDef = serde_json::from_value(json!({
            "nodes": {
                "draft": {"agent": {}, "input": {"from": "run_input"}},
                "critic": {"agent": {}, "input": {"from": "node:draft"}, "depends_on": ["draft"],
                           "fanout": {"items": ["security", "cost"]}},
                "final": {"agent": {}, "input": {"from": "node:critic"}, "depends_on": ["critic"]}
            },
            "output": {"from": "node:final"}
        }))
        .unwrap();
        if !reads {
            let critic = d.nodes.get_mut("critic").unwrap();
            critic.input.from = "fanout_item".into();
            critic.depends_on.clear();
        }
        d
    }

    #[test]
    fn written_array_expands_at_once_without_reads() {
        let d = critics_def(false);
        let mut r = record();
        assert_eq!(
            expand_ready_fanouts(&d, &mut r, &BTreeMap::new()),
            vec!["critic".to_string()]
        );
        assert_eq!(
            r.fanout_src["critic"],
            vec![json!("security"), json!("cost")]
        );
        assert_eq!(
            fanned_uids(&r, "critic"),
            vec!["critic#0".to_string(), "critic#1".to_string()]
        );
    }

    #[test]
    fn written_array_waits_for_its_reads() {
        let d = critics_def(true);
        let mut r = record();
        assert!(expand_ready_fanouts(&d, &mut r, &BTreeMap::new()).is_empty());
        r.nodes.insert("draft".into(), done_checkpoint());
        assert_eq!(
            expand_ready_fanouts(&d, &mut r, &BTreeMap::new()),
            vec!["critic".to_string()]
        );
    }

    #[test]
    fn path_fanout_with_reads_waits_for_source_and_reads() {
        let mut d = def();
        d.nodes.insert(
            "spec".into(),
            serde_json::from_value(json!({"agent": {}, "input": {"from": "run_input"}})).unwrap(),
        );
        let read = d.nodes.get_mut("read").unwrap();
        read.input.from = "node:spec".into();
        read.depends_on.push("spec".into());
        let mut r = record();
        let results = BTreeMap::from([("plan".to_string(), json!({"docs": ["a"]}))]);
        r.nodes.insert("plan".into(), done_checkpoint());
        assert!(expand_ready_fanouts(&d, &mut r, &results).is_empty());
        r.nodes.insert("spec".into(), done_checkpoint());
        assert_eq!(
            expand_ready_fanouts(&d, &mut r, &results),
            vec!["read".to_string()]
        );
    }

    #[test]
    fn written_array_over_the_cap_names_its_own_array() {
        let mut d = critics_def(false);
        d.nodes
            .get_mut("critic")
            .unwrap()
            .fanout
            .as_mut()
            .unwrap()
            .items = Some(vec![json!(0); MAX_FANOUT_ITEMS + 1]);
        let mut r = record();
        expand_ready_fanouts(&d, &mut r, &BTreeMap::new());
        assert_eq!(
            r.nodes["critic"].result_error.as_deref(),
            Some("fanout 'critic' over its own array: array has 10001 items, exceeds the cap of 10000")
        );
    }

    #[test]
    fn reader_of_written_array_fanout_gets_results_in_written_order() {
        let d = critics_def(true);
        let mut r = record();
        r.nodes.insert("draft".into(), done_checkpoint());
        expand_ready_fanouts(&d, &mut r, &BTreeMap::new());
        let results = BTreeMap::from([
            ("critic#1".to_string(), json!("on cost")),
            ("critic#0".to_string(), json!("on security")),
        ]);
        assert_eq!(
            gather_input(&d, &r, "final", &results),
            json!(["on security", "on cost"])
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

    fn pending_send_retry() -> NodeCheckpoint {
        NodeCheckpoint {
            session_id: Some("wf_run_test_b@r0".into()),
            result_error: Some("send failed: invocation timed out".into()),
            send_failed_at: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn ready_frontier_includes_pending_checkpoint_of_normal_node() {
        let d = def_with_orphan();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), pending_send_retry());
        r.nodes.insert("orphan".into(), running_checkpoint());
        assert_eq!(ready_frontier(&d, &r), vec!["b".to_string()]);
    }

    #[test]
    fn quiescence_does_not_fail_on_pending_with_error() {
        let d = def_with_orphan();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), pending_send_retry());
        assert_eq!(quiescence(&d, &r), RunStatus::AwaitingNodes);
    }

    // -----------------------------------------------------------------------
    // on_error: continue, and output.from as an array
    // -----------------------------------------------------------------------

    fn continue_on(d: &mut WorkflowDef, ids: &[&str]) {
        for id in ids {
            d.nodes.get_mut(*id).unwrap().on_error = OnError::Continue;
        }
    }

    fn cancelled_checkpoint() -> NodeCheckpoint {
        NodeCheckpoint {
            state: NodeState::Cancelled,
            result_error: Some("cancelled".into()),
            ..Default::default()
        }
    }

    /// Three independent text nodes x, y, z plus an orphan; `output.from` as given.
    fn independent_def(output_from: Value) -> WorkflowDef {
        serde_json::from_value(json!({
            "output": {"from": output_from},
            "nodes": {
                "x": {"agent": {"model": "m"}, "input": {"from": "run_input"}},
                "y": {"agent": {"model": "m"}, "input": {"from": "run_input"}},
                "z": {"agent": {"model": "m"}, "input": {"from": "run_input"}},
                "orphan": {"agent": {"model": "m"}, "input": {"from": "run_input"}}
            }
        }))
        .unwrap()
    }

    #[test]
    fn deps_done_true_when_failed_dep_continues() {
        let mut d = def_with_orphan();
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), failed_checkpoint());
        assert!(!deps_done(&d, &r, "d"), "a strict failed dep blocks");
        continue_on(&mut d, &["b"]);
        assert!(deps_done(&d, &r, "d"));
        assert!(ready_frontier(&d, &r).contains(&"d".to_string()));
    }

    #[test]
    fn cancelled_node_with_continue_settles() {
        assert!(is_settled(NodeState::Cancelled, true));
        assert!(!is_settled(NodeState::Cancelled, false));
        assert!(!is_settled(NodeState::Running, true));
        assert!(!is_settled(NodeState::Pending, true));
        let mut d = def_with_orphan();
        continue_on(&mut d, &["b"]);
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), cancelled_checkpoint());
        assert!(deps_done(&d, &r, "d"));
    }

    #[test]
    fn quiescence_completes_when_required_failure_continues() {
        let mut d = def_with_orphan();
        continue_on(&mut d, &["b"]);
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), failed_checkpoint());
        assert_eq!(
            quiescence(&d, &r),
            RunStatus::AwaitingNodes,
            "d still to run"
        );
        r.nodes.insert("d".into(), running_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::AwaitingNodes);
        r.nodes.insert("d".into(), done_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);

        // The output node itself failing with continue also completes the run.
        continue_on(&mut d, &["d"]);
        r.nodes.insert("d".into(), failed_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);
    }

    #[test]
    fn quiescence_tolerates_failed_fanout_item_with_continue() {
        let mut d = def();
        continue_on(&mut d, &["read"]);
        let mut r = record();
        r.nodes.insert("plan".into(), done_checkpoint());
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs": ["a", "b", "c"]}));
        expand_ready_fanouts(&d, &mut r, &results);
        r.nodes.insert("read#0".into(), done_checkpoint());
        r.nodes.insert("read#1".into(), failed_checkpoint());
        r.nodes.insert("read#2".into(), running_checkpoint());
        results.insert("read#0".into(), json!("v0"));
        assert!(!deps_done(&d, &r, "synthesize"), "read#2 still running");
        assert_eq!(quiescence(&d, &r), RunStatus::AwaitingNodes);

        r.nodes.insert("read#2".into(), done_checkpoint());
        results.insert("read#2".into(), json!("v2"));
        assert!(deps_done(&d, &r, "synthesize"));
        assert_eq!(
            gather_input(&d, &r, "synthesize", &results),
            json!(["v0", null, "v2"]),
            "fan-in keeps positions"
        );
        r.nodes.insert("synthesize".into(), done_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);
    }

    #[test]
    fn abort_wins_over_continue() {
        let mut d = def_with_orphan();
        continue_on(&mut d, &["a", "b", "d"]);
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), failed_checkpoint());
        r.abort = true;
        assert_eq!(quiescence(&d, &r), RunStatus::Cancelled);
    }

    #[test]
    fn gather_input_yields_null_for_failed_continue_dep() {
        let mut d = def_with_orphan();
        continue_on(&mut d, &["b"]);
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        r.nodes.insert("b".into(), failed_checkpoint());
        let mut results = BTreeMap::new();
        results.insert("a".to_string(), json!({"k": 1}));
        assert_eq!(gather_input(&d, &r, "d", &results), Value::Null);
        assert_ne!(gather_input(&d, &r, "d", &results), r.input);
    }

    #[test]
    fn gather_input_reads_text_dep_whole_as_string() {
        let mut d = def_with_orphan();
        d.nodes.get_mut("a").unwrap().agent.output = None;
        let mut r = record();
        r.nodes.insert("a".into(), done_checkpoint());
        let text = "Report.\n- point one\n- point two";
        let results = BTreeMap::from([("a".to_string(), json!(text))]);
        assert_eq!(gather_input(&d, &r, "b", &results), json!(text));
        d.nodes.get_mut("b").unwrap().input.from = InputFrom::Many(vec!["node:a".into()]);
        assert_eq!(gather_input(&d, &r, "b", &results), json!({ "a": text }));
    }

    #[test]
    fn failed_expansion_with_continue_gives_empty_array() {
        let mut d = empty_fanout_def();
        continue_on(&mut d, &["a", "b"]);
        let mut r = record();
        r.nodes.insert("a".into(), failed_checkpoint());
        let results = BTreeMap::new();
        assert_eq!(expand_ready_fanouts(&d, &mut r, &results), vec!["b"]);
        assert_eq!(
            r.nodes["b"].state,
            NodeState::Failed,
            "no result to fan over"
        );
        assert!(deps_done(&d, &r, "c"));
        assert_eq!(gather_input(&d, &r, "c", &results), json!([]));
        assert_eq!(
            quiescence(&d, &r),
            RunStatus::AwaitingNodes,
            "c still to run"
        );
        r.nodes.insert("c".into(), done_checkpoint());
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);
    }

    #[test]
    fn tolerant_source_with_strict_fanout_consumer_fails_run() {
        let mut d = empty_fanout_def();
        continue_on(&mut d, &["a"]);
        let mut r = record();
        r.nodes.insert("a".into(), failed_checkpoint());
        expand_ready_fanouts(&d, &mut r, &BTreeMap::new());
        assert_eq!(r.nodes["b"].state, NodeState::Failed);
        assert!(!deps_done(&d, &r, "c"));
        assert_eq!(quiescence(&d, &r), RunStatus::Failed);
    }

    #[test]
    fn required_set_unions_output_array() {
        let d = independent_def(json!(["node:x", "y"]));
        let req = required_set(&d);
        assert_eq!(req, BTreeSet::from(["x".to_string(), "y".to_string()]));
        let d = def_with_orphan();
        let mut d2 = d.clone();
        d2.output.from = OutputFrom::Many(vec!["node:d".into(), "node:orphan".into()]);
        let req = required_set(&d2);
        for id in ["a", "b", "d", "orphan"] {
            assert!(req.contains(id), "{id}");
        }
        assert!(!required_set(&d).contains("orphan"));
    }

    #[test]
    fn gather_output_many_is_object_keyed_by_node() {
        let mut d = def();
        d.output.from = OutputFrom::Many(vec!["node:plan".into(), "node:read".into()]);
        let mut r = record();
        r.fanout_src.insert("read".into(), vec![json!(0), json!(1)]);
        let mut results = BTreeMap::new();
        results.insert("plan".to_string(), json!({"docs": [0, 1]}));
        results.insert("read#1".to_string(), json!("r1"));
        assert_eq!(
            gather_output(&d, &r, &results),
            json!({"plan": {"docs": [0, 1]}, "read": [null, "r1"]})
        );
    }

    #[test]
    fn gather_output_bare_id_matches_node_prefix() {
        let mut results = BTreeMap::new();
        results.insert("y".to_string(), json!("text y"));
        let r = record();
        let bare = gather_output(&independent_def(json!("y")), &r, &results);
        let prefixed = gather_output(&independent_def(json!("node:y")), &r, &results);
        assert_eq!(bare, json!("text y"));
        assert_eq!(bare, prefixed);
        // A bare id with no result is null, never the run input.
        let missing = gather_output(&independent_def(json!(["x"])), &r, &results);
        assert_eq!(missing, json!({"x": null}));
    }

    #[test]
    fn gather_output_one_keeps_bare_result() {
        let mut d = def();
        let mut r = record();
        r.fanout_src.insert("read".into(), vec![json!(0)]);
        let mut results = BTreeMap::new();
        results.insert("synthesize".to_string(), json!({"summary": "s"}));
        results.insert("read#0".to_string(), json!("r0"));
        assert_eq!(gather_output(&d, &r, &results), json!({"summary": "s"}));
        d.output.from = "node:read".into();
        assert_eq!(gather_output(&d, &r, &results), json!(["r0"]));
    }

    // -----------------------------------------------------------------------
    // Loops: w (work, text) → v (verify, json, loop body [w]) → out
    // -----------------------------------------------------------------------

    fn loop_def(max_repeats: u32) -> WorkflowDef {
        serde_json::from_value(json!({
            "nodes": {
                "w": {"agent": {"model": "m"}, "input": {"from": "run_input"}},
                "v": {
                    "agent": {"model": "m", "output": {"type": "json"}},
                    "input": {"from": "node:w"}, "depends_on": ["w"],
                    "loop": {"body": ["w"], "while": {"field": "verdict", "equals": "reject"},
                             "max_repeats": max_repeats}
                },
                "out": {"agent": {"model": "m"}, "input": {"from": "node:v"}, "depends_on": ["v"]}
            },
            "output": {"from": "node:out"}
        }))
        .expect("loop def")
    }

    fn set_done(
        r: &mut WorkflowRunRecord,
        results: &mut BTreeMap<String, Value>,
        uid: &str,
        value: Value,
    ) {
        r.nodes.insert(uid.to_string(), done_checkpoint());
        results.insert(uid.to_string(), value);
    }

    /// Pass 0 of `w` and `v` Done, `v` reading `verdict`.
    fn first_pass_done(verdict: &str) -> (WorkflowRunRecord, BTreeMap<String, Value>) {
        let mut r = record();
        let mut results = BTreeMap::new();
        set_done(&mut r, &mut results, "w", json!("work 0"));
        set_done(&mut r, &mut results, "v", json!({"verdict": verdict}));
        (r, results)
    }

    #[test]
    fn condition_holds_compares_field_by_equality() {
        let cond = LoopCondition {
            field: "review.verdict".into(),
            equals: "reject".into(),
        };
        let holds = |v: Value| condition_holds(&cond, Some(&v));
        assert!(holds(json!({"review": {"verdict": "reject"}})));
        assert!(!holds(json!({"review": {"verdict": "pass"}})));
        assert!(!holds(json!({"review": {}})), "missing field");
        assert!(
            !holds(json!({"review": "reject"})),
            "not an object on the way"
        );
        assert!(!holds(json!(null)), "null result");
        assert!(!condition_holds(&cond, None), "no result");
        let flag = LoopCondition {
            field: "ok".into(),
            equals: "true".into(),
        };
        assert!(
            !condition_holds(&flag, Some(&json!({"ok": true}))),
            "a non-string value never equals the string"
        );
    }

    #[test]
    fn loop_expands_next_pass_when_condition_holds() {
        let d = loop_def(2);
        let (mut r, results) = first_pass_done("reject");
        assert_eq!(expand_ready_loops(&d, &mut r, &results), vec!["v"]);
        for uid in ["w#it1", "v#it1"] {
            let cp = r.nodes.get(uid).expect(uid);
            assert_eq!(cp.state, NodeState::Pending);
            assert!(cp.session_id.is_none(), "{uid} holds no session yet");
        }
        assert!(!r.nodes.contains_key("out"));
        assert_eq!(last_uid(&r, "v"), "v#it1");
        assert_eq!(last_uid(&r, "w"), "w#it1");
        assert_eq!(last_uid(&r, "out"), "out");
        // An explicit pass and a fanout item stay put.
        assert_eq!(last_uid(&r, "v#it1"), "v#it1");
        assert_eq!(last_uid(&r, "read#0"), "read#0");
    }

    #[test]
    fn loop_stops_when_condition_false_or_repeats_exhausted() {
        let d = loop_def(1);
        let (mut r, results) = first_pass_done("pass");
        assert!(expand_ready_loops(&d, &mut r, &results).is_empty());
        assert_eq!(r.nodes.len(), 2);

        let (mut r, mut results) = first_pass_done("reject");
        assert_eq!(expand_ready_loops(&d, &mut r, &results), vec!["v"]);
        set_done(&mut r, &mut results, "w#it1", json!("work 1"));
        set_done(&mut r, &mut results, "v#it1", json!({"verdict": "reject"}));
        assert!(
            expand_ready_loops(&d, &mut r, &results).is_empty(),
            "max_repeats 1 allows one pass after the first"
        );
        assert!(!r.nodes.contains_key("v#it2"));
    }

    #[test]
    fn loop_stops_when_body_pass_not_done() {
        let mut d = loop_def(2);
        d.nodes.get_mut("w").unwrap().on_error = OnError::Continue;
        for state in [NodeState::Running, NodeState::Failed, NodeState::Cancelled] {
            let (mut r, results) = first_pass_done("reject");
            r.nodes.get_mut("w").unwrap().state = state;
            assert!(
                expand_ready_loops(&d, &mut r, &results).is_empty(),
                "{state:?} body pass ends the loop, even when tolerated"
            );
        }
        // The loop node itself has not finished.
        let (mut r, results) = first_pass_done("reject");
        r.nodes.get_mut("v").unwrap().state = NodeState::Running;
        assert!(expand_ready_loops(&d, &mut r, &results).is_empty());
    }

    #[test]
    fn loop_expansion_is_idempotent() {
        let d = loop_def(2);
        let (mut r, results) = first_pass_done("reject");
        expand_ready_loops(&d, &mut r, &results);
        let after_first = r.clone();
        assert!(expand_ready_loops(&d, &mut r, &results).is_empty());
        assert_eq!(r, after_first);
    }

    #[test]
    fn loop_stops_at_run_node_cap_without_partial_pass() {
        let d = loop_def(2);
        let fill = |r: &mut WorkflowRunRecord, total: usize| {
            let mut i = 0;
            while r.nodes.len() < total {
                r.nodes.insert(format!("filler{i}"), done_checkpoint());
                i += 1;
            }
        };
        let (mut r, results) = first_pass_done("reject");
        fill(&mut r, MAX_TOTAL_NODES - 1);
        assert!(expand_ready_loops(&d, &mut r, &results).is_empty());
        assert!(!r.nodes.contains_key("w#it1") && !r.nodes.contains_key("v#it1"));

        let (mut r, results) = first_pass_done("reject");
        fill(&mut r, MAX_TOTAL_NODES - 2);
        assert_eq!(expand_ready_loops(&d, &mut r, &results), vec!["v"]);
        assert_eq!(r.nodes.len(), MAX_TOTAL_NODES);
    }

    #[test]
    fn pass_deps_wait_only_for_the_same_pass() {
        let d = loop_def(2);
        let (mut r, mut results) = first_pass_done("reject");
        expand_ready_loops(&d, &mut r, &results);
        assert!(deps_done(&d, &r, "w#it1"), "w reads only run_input");
        assert!(
            !deps_done(&d, &r, "v#it1"),
            "v#it1 waits for w#it1, not the Done first pass"
        );
        assert!(deps_done(&d, &r, "v"), "pass 0 still reads pass 0");
        set_done(&mut r, &mut results, "w#it1", json!("work 1"));
        assert!(deps_done(&d, &r, "v#it1"));
        assert_eq!(gather_input(&d, &r, "v#it1", &results), json!("work 1"));
        assert_eq!(gather_input(&d, &r, "v", &results), json!("work 0"));
    }

    #[test]
    fn frontier_lists_pending_passes() {
        let d = loop_def(2);
        let (mut r, mut results) = first_pass_done("reject");
        expand_ready_loops(&d, &mut r, &results);
        assert_eq!(ready_frontier(&d, &r), vec!["w#it1"]);
        r.nodes.get_mut("w#it1").unwrap().state = NodeState::Running;
        assert!(ready_frontier(&d, &r).is_empty());
        set_done(&mut r, &mut results, "w#it1", json!("work 1"));
        assert_eq!(ready_frontier(&d, &r), vec!["v#it1"]);
    }

    #[test]
    fn outside_consumer_waits_for_and_reads_last_pass() {
        let mut d = loop_def(2);
        let (mut r, mut results) = first_pass_done("reject");
        expand_ready_loops(&d, &mut r, &results);
        assert!(!deps_done(&d, &r, "out"), "out waits for v#it1");
        assert!(!ready_frontier(&d, &r).contains(&"out".to_string()));
        set_done(&mut r, &mut results, "w#it1", json!("work 1"));
        set_done(&mut r, &mut results, "v#it1", json!({"verdict": "pass"}));
        assert!(expand_ready_loops(&d, &mut r, &results).is_empty());
        assert!(deps_done(&d, &r, "out"));
        assert_eq!(
            gather_input(&d, &r, "out", &results),
            json!({"verdict": "pass"})
        );
        d.output.from = "node:v".into();
        assert_eq!(gather_output(&d, &r, &results), json!({"verdict": "pass"}));
    }

    #[test]
    fn loop_previous_is_null_first_then_previous_result() {
        let d = loop_def(2);
        let (mut r, mut results) = first_pass_done("reject");
        assert_eq!(
            loop_previous(&d, "w", &results),
            Some((json!(null), Some(json!(null))))
        );
        assert_eq!(loop_previous(&d, "v", &results), Some((json!(null), None)));
        assert_eq!(loop_previous(&d, "out", &results), None, "not in a loop");
        expand_ready_loops(&d, &mut r, &results);
        let first = json!({"verdict": "reject"});
        assert_eq!(
            loop_previous(&d, "w#it1", &results),
            Some((first.clone(), Some(json!("work 0"))))
        );
        assert_eq!(loop_previous(&d, "v#it1", &results), Some((first, None)));
        set_done(&mut r, &mut results, "v#it1", json!({"verdict": "again"}));
        set_done(&mut r, &mut results, "w#it1", json!("work 1"));
        assert_eq!(
            loop_previous(&d, "w#it2", &results),
            Some((json!({"verdict": "again"}), Some(json!("work 1"))))
        );
    }

    #[test]
    fn quiescence_awaits_pending_pass() {
        let mut d = loop_def(2);
        d.output.from = "node:v".into();
        let (mut r, mut results) = first_pass_done("reject");
        expand_ready_loops(&d, &mut r, &results);
        assert_eq!(quiescence(&d, &r), RunStatus::AwaitingNodes);
        set_done(&mut r, &mut results, "w#it1", json!("work 1"));
        set_done(&mut r, &mut results, "v#it1", json!({"verdict": "pass"}));
        assert_eq!(quiescence(&d, &r), RunStatus::Completed);
        // A strict failure in a later pass fails the run.
        r.nodes.get_mut("w#it1").unwrap().state = NodeState::Failed;
        assert_eq!(quiescence(&d, &r), RunStatus::Failed);
    }
}
