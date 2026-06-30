use std::collections::BTreeMap;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{from_value, json, Value};

use crate::{
    error::WorkflowError,
    ids::{def_key, node_result_key},
    types::{NodeState, WorkflowDef, WorkflowRunRecord},
};

// ---------------------------------------------------------------------------
// Scope constants
// ---------------------------------------------------------------------------

pub const SCOPE_RUN: &str = "workflow_run";
pub const SCOPE_DEF: &str = "workflow_def";
pub const SCOPE_RESULT: &str = "workflow_node_result";
pub const SCOPE_INDEX: &str = "workflow_session_index";
pub const SCOPE_IDEM: &str = "workflow_idem";

// RPC timeout for state::* dispatch. Process-global so it can be wired from
// WorkerConfig.dispatch_timeout_ms without threading a param through every state
// call. ponytail: set at boot + on config change; relaxed ordering is fine for a
// tuning knob; per-call threading if a run ever needs its own timeout.
static DISPATCH_TIMEOUT_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(30_000);

/// Wire the state-layer RPC dispatch timeout from config (boot + hot-reload).
pub fn set_dispatch_timeout_ms(ms: u64) {
    DISPATCH_TIMEOUT_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

fn dispatch_timeout_ms() -> u64 {
    DISPATCH_TIMEOUT_MS.load(std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Private primitive helpers
// ---------------------------------------------------------------------------

async fn state_get(iii: &IIIClient, scope: &str, key: &str) -> Result<Value, WorkflowError> {
    iii.trigger(TriggerRequest {
        function_id: "state::get".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(dispatch_timeout_ms()),
    })
    .await
    .map_err(|e| WorkflowError::State(format!("state::get {scope}/{key}: {e}")))
}

async fn state_set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: Value,
) -> Result<(), WorkflowError> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".into(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(dispatch_timeout_ms()),
    })
    .await
    .map(|_| ())
    .map_err(|e| WorkflowError::State(format!("state::set {scope}/{key}: {e}")))
}

async fn state_delete(iii: &IIIClient, scope: &str, key: &str) -> Result<(), WorkflowError> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".into(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(dispatch_timeout_ms()),
    })
    .await
    .map(|_| ())
    .map_err(|e| WorkflowError::State(format!("state::delete {scope}/{key}: {e}")))
}

async fn state_list(iii: &IIIClient, scope: &str) -> Result<Value, WorkflowError> {
    iii.trigger(TriggerRequest {
        function_id: "state::list".into(),
        payload: json!({ "scope": scope }),
        action: None,
        timeout_ms: Some(dispatch_timeout_ms()),
    })
    .await
    .map_err(|e| WorkflowError::State(format!("state::list {scope}: {e}")))
}

// ---------------------------------------------------------------------------
// Parse helpers (pure — unit-testable)
// ---------------------------------------------------------------------------

/// Tolerates three shapes returned by the state engine:
/// 1. Bare array: `[...]`
/// 2. Object with `values` or `items` key: `{"values":[...]}` / `{"items":[...]}`
/// 3. Key→value map: `{"r_1": {...}, "r_2": {...}}`
pub fn parse_record_list(v: &Value) -> Vec<WorkflowRunRecord> {
    let items: Vec<Value> = if let Some(arr) = v.as_array() {
        arr.clone()
    } else if let Some(obj) = v.as_object() {
        if let Some(arr) = obj.get("values").and_then(|x| x.as_array()) {
            arr.clone()
        } else if let Some(arr) = obj.get("items").and_then(|x| x.as_array()) {
            arr.clone()
        } else {
            // key → value map
            obj.values().cloned().collect()
        }
    } else {
        return vec![];
    };

    items
        .into_iter()
        .filter_map(|item| from_value::<WorkflowRunRecord>(item).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Run record
// ---------------------------------------------------------------------------

pub async fn get_run(
    iii: &IIIClient,
    run_id: &str,
) -> Result<Option<WorkflowRunRecord>, WorkflowError> {
    let v = state_get(iii, SCOPE_RUN, run_id).await?;
    if v.is_null() {
        Ok(None)
    } else {
        Ok(Some(from_value(v)?))
    }
}

/// Persist a run record.
///
/// **Single-writer-per-run is the correctness contract.** Callers MUST hold
/// the per-run `WorkflowLocks` guard before calling this function.  The lock
/// serializes every read-modify-write path (tick, reconcile, sweep, stop) for
/// a given `run_id` within one process so writes never clobber each other.
///
/// NOTE: `state::set` is an unconditional overwrite — iii-state (sdk 0.19.2)
/// exposes no compare-and-set.  A multi-process deployment therefore requires
/// run-level sharding (route all `workflow::tick` events for a `run_id` to one
/// owning instance) rather than record-level CAS.  See README §Scaling for the
/// supported multi-instance topology.
pub async fn put_run(iii: &IIIClient, record: &WorkflowRunRecord) -> Result<(), WorkflowError> {
    let v = serde_json::to_value(record)?;
    state_set(iii, SCOPE_RUN, &record.run_id, v).await
}

pub async fn list_runs(iii: &IIIClient) -> Result<Vec<WorkflowRunRecord>, WorkflowError> {
    let v = state_list(iii, SCOPE_RUN).await?;
    Ok(parse_record_list(&v))
}

/// Delete a terminal run's persisted state: its node-result blobs, then its
/// definition, then the run record itself. Used by the sweep to GC runs past the
/// retention window so `list_runs` doesn't deserialize unbounded history every
/// cycle. The run record is deleted LAST so a mid-delete crash leaves the next
/// sweep able to retry (the record still lists the result/def keys). The session
/// reverse-index is intentionally left as-is: a stale entry resolves to a now-missing
/// run, which `run_id_for_session` callers already tolerate (get_run → None ends the
/// ancestry walk).
pub async fn delete_run(iii: &IIIClient, record: &WorkflowRunRecord) -> Result<(), WorkflowError> {
    for cp in record.nodes.values() {
        if let Some(key) = &cp.result_ref {
            let _ = state_delete(iii, SCOPE_RESULT, key).await;
        }
    }
    let _ = state_delete(iii, SCOPE_DEF, &def_key(&record.run_id)).await;
    state_delete(iii, SCOPE_RUN, &record.run_id).await
}

// ---------------------------------------------------------------------------
// Workflow definition
// ---------------------------------------------------------------------------

pub async fn put_def(
    iii: &IIIClient,
    run_id: &str,
    def: &WorkflowDef,
) -> Result<(), WorkflowError> {
    let key = def_key(run_id);
    let v = serde_json::to_value(def)?;
    state_set(iii, SCOPE_DEF, &key, v).await
}

pub async fn get_def(iii: &IIIClient, run_id: &str) -> Result<Option<WorkflowDef>, WorkflowError> {
    let key = def_key(run_id);
    let v = state_get(iii, SCOPE_DEF, &key).await?;
    if v.is_null() {
        Ok(None)
    } else {
        Ok(Some(from_value(v)?))
    }
}

// ---------------------------------------------------------------------------
// Node results
// ---------------------------------------------------------------------------

pub async fn put_node_result(
    iii: &IIIClient,
    run_id: &str,
    node_uid: &str,
    result: &Value,
) -> Result<(), WorkflowError> {
    let key = node_result_key(run_id, node_uid);
    state_set(iii, SCOPE_RESULT, &key, result.clone()).await
}

pub async fn get_node_result(
    iii: &IIIClient,
    run_id: &str,
    node_uid: &str,
) -> Result<Option<Value>, WorkflowError> {
    let key = node_result_key(run_id, node_uid);
    let v = state_get(iii, SCOPE_RESULT, &key).await?;
    if v.is_null() {
        Ok(None)
    } else {
        Ok(Some(v))
    }
}

/// For each `NodeState::Done` checkpoint that has a `result_ref`, fetch the
/// stored result and collect into a `node_uid → result` map.
pub async fn load_done_results(
    iii: &IIIClient,
    record: &mut WorkflowRunRecord,
) -> Result<BTreeMap<String, Value>, WorkflowError> {
    let mut out = BTreeMap::new();
    let mut corrupt: Vec<String> = Vec::new();
    for (node_uid, checkpoint) in &record.nodes {
        if checkpoint.state == NodeState::Done && checkpoint.result_ref.is_some() {
            match get_node_result(iii, &record.run_id, node_uid).await? {
                Some(result) => {
                    out.insert(node_uid.clone(), result);
                }
                None => corrupt.push(node_uid.clone()),
            }
        }
    }
    // A Done node always has its result persisted before it is marked Done
    // (reconcile). A missing one is state corruption. Do NOT hard-error: that
    // fails EVERY tick for this run forever — the run wedges in AwaitingNodes and
    // the sweep re-enqueues a dying tick each cycle. Instead mark the node Failed
    // so `quiescence` fails the run cleanly with a diagnosable message, rather
    // than feeding Null downstream (which would derail gather_input / fanout).
    for node_uid in corrupt {
        if let Some(cp) = record.nodes.get_mut(&node_uid) {
            cp.state = NodeState::Failed;
            cp.result_error = Some(
                "Done checkpoint has a result_ref but the stored result is missing \
                 (state corruption)"
                    .to_string(),
            );
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Session reverse index (session_id -> run_id)
// ---------------------------------------------------------------------------
//
// Written when a node is fired; read by `workflow::wake` to map a harness
// `turn-completed` event back to its owning run. Only `run_id` is stored —
// waking re-ticks the whole run, so the node_uid isn't needed.

pub async fn put_session_index(
    iii: &IIIClient,
    session_id: &str,
    run_id: &str,
) -> Result<(), WorkflowError> {
    state_set(iii, SCOPE_INDEX, session_id, json!(run_id)).await
}

pub async fn run_id_for_session(
    iii: &IIIClient,
    session_id: &str,
) -> Result<Option<String>, WorkflowError> {
    let v = state_get(iii, SCOPE_INDEX, session_id).await?;
    if v.is_null() {
        Ok(None)
    } else {
        let s = v
            .as_str()
            .ok_or_else(|| WorkflowError::State(format!("session index not a string: {v}")))?
            .to_string();
        Ok(Some(s))
    }
}

// ---------------------------------------------------------------------------
// Idempotency keys
// ---------------------------------------------------------------------------

pub async fn get_idem(iii: &IIIClient, key: &str) -> Result<Option<String>, WorkflowError> {
    let v = state_get(iii, SCOPE_IDEM, key).await?;
    if v.is_null() {
        Ok(None)
    } else {
        let s = v
            .as_str()
            .ok_or_else(|| WorkflowError::State(format!("idem value not a string: {v}")))?
            .to_string();
        Ok(Some(s))
    }
}

pub async fn put_idem(iii: &IIIClient, key: &str, run_id: &str) -> Result<(), WorkflowError> {
    state_set(iii, SCOPE_IDEM, key, json!(run_id)).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_record_json() -> Value {
        json!({
            "run_id": "r_1",
            "step": 0,
            "status": "running",
            "def_ref": "r_1",
            "input": {},
            "created_at": 1,
            "updated_at": 1
        })
    }

    #[test]
    fn scopes_are_flat_strings() {
        assert_eq!(SCOPE_RUN, "workflow_run");
        assert_eq!(SCOPE_RESULT, "workflow_node_result");
        assert_eq!(SCOPE_INDEX, "workflow_session_index");
        // Verify they are never "workflow_run/<id>" style — no slashes
        assert!(!SCOPE_RUN.contains('/'));
        assert!(!SCOPE_DEF.contains('/'));
        assert!(!SCOPE_RESULT.contains('/'));
        assert!(!SCOPE_INDEX.contains('/'));
        assert!(!SCOPE_IDEM.contains('/'));
    }

    #[test]
    fn parse_record_list_tolerates_array_object_and_map() {
        let rec = minimal_record_json();

        // Shape 1: bare array
        let v1 = json!([rec.clone()]);
        let list1 = parse_record_list(&v1);
        assert_eq!(list1.len(), 1, "bare array should yield 1 record");
        assert_eq!(list1[0].run_id, "r_1");

        // Shape 2: object with "values" key
        let v2 = json!({ "values": [rec.clone()] });
        let list2 = parse_record_list(&v2);
        assert_eq!(list2.len(), 1, "{{values:[...]}} should yield 1 record");
        assert_eq!(list2[0].run_id, "r_1");

        // Shape 2b: object with "items" key
        let v2b = json!({ "items": [rec.clone()] });
        let list2b = parse_record_list(&v2b);
        assert_eq!(list2b.len(), 1, "{{items:[...]}} should yield 1 record");
        assert_eq!(list2b[0].run_id, "r_1");

        // Shape 3: key→value map
        let v3 = json!({ "r_1": rec.clone() });
        let list3 = parse_record_list(&v3);
        assert_eq!(list3.len(), 1, "key→value map should yield 1 record");
        assert_eq!(list3[0].run_id, "r_1");
    }
}
