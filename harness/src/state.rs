//! Durable loop bookkeeping in iii state (harness.md § State).
//!
//! Four scopes: `harness_turn/<session_id>` holds the [`TurnRecord`] (loop
//! progress, per-send options, per-call checkpoints),
//! `harness_idem/<idempotency_key>` holds the webhook-dedupe row,
//! `harness_queue/<session_id>:<id>` holds one [`QueuedMessage`] per message
//! that arrived while a step was streaming (drained by the loop), and
//! `harness_binding/<binding_id>` holds one [`crate::bindings::Binding`].
//! Binding scopes use the state worker's hidden harness API; ordinary
//! bookkeeping keeps the public `state::*` compatibility surface.

use std::collections::{BTreeMap, BTreeSet};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::HarnessError;
use crate::trace_tags::run_hidden;
use crate::types::message::AgentMessage;
use crate::types::turn::{IdemRecord, TurnRecord, TurnStatus};

pub const TURN_SCOPE: &str = "harness_turn";
pub const IDEM_SCOPE: &str = "harness_idem";
pub const QUEUE_SCOPE: &str = "harness_queue";
/// Per-session anonymous-usage bookkeeping (`usage_report::UsageRow`): which
/// milestones and outcomes this session has already announced.
pub const USAGE_SCOPE: &str = "harness_usage";
/// One durable trigger binding per key (`bindings::Binding`). The engine-side
/// trigger metadata holds only this key; everything the fire needs is here.
pub const BINDING_SCOPE: &str = "harness_binding";
/// Per-owner binding ids. Updated with CAS so capacity reservation and binding
/// insertion are one logical operation across concurrent harness workers.
pub const BINDING_OWNER_SCOPE: &str = "harness_binding_owner";

const STATE_GET_ID: &str = "state::get";
const STATE_SET_ID: &str = "state::set";
const STATE_DELETE_ID: &str = "state::delete";
/// Strict list reader for lifecycle safety: malformed state must not disappear.
pub(crate) async fn list_values<T: serde::de::DeserializeOwned>(
    iii: &IIIClient,
    scope: &str,
    timeout_ms: u64,
) -> Result<Vec<T>, HarnessError> {
    let value = state_list(iii, scope, timeout_ms).await?;
    let values: Vec<Value> = match value {
        Value::Array(values) => values,
        Value::Object(mut map) => match map.remove("values").or_else(|| map.remove("items")) {
            Some(Value::Array(values)) => values,
            Some(_) => return Err(HarnessError::State(format!("malformed list {scope}"))),
            None => map.into_values().collect(),
        },
        _ => return Err(HarnessError::State(format!("malformed list {scope}"))),
    };
    values
        .into_iter()
        .filter(|v| !v.is_null())
        .map(|v| {
            serde_json::from_value(v)
                .map_err(|e| HarnessError::State(format!("malformed {scope} row: {e}")))
        })
        .collect()
}

const STATE_LIST_ID: &str = "state::list";
const STATE_LIST_KEYS_ID: &str = "state::list_keys";
const PRIVATE_STATE_GET_ID: &str = "harness::state::get";
const PRIVATE_STATE_LIST_ID: &str = "harness::state::list";
const STATE_CAS_ID: &str = "harness::state::compare-and-set";
/// The state worker owns no harness knowledge: this worker CLAIMS its private
/// namespace at runtime and the accessors above are registered in response.
pub(crate) const CLAIM_NAMESPACE_ID: &str = "state::claim-namespace";
/// Our namespace is our worker name — the only one the state worker will let
/// us claim (it authorizes on the engine-stamped caller identity).
const NAMESPACE_PREFIX: &str = "harness";

/// Every `state::*` call below is loop bookkeeping that fires several times
/// per turn step — tagged `iii.tag.hidden` so trace UIs stack the spans into
/// the span filter's internal section, hidden by default.
const HIDDEN_FAMILY: &str = "harness state";

pub(crate) async fn state_get(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Result<Value, HarnessError> {
    let private = is_binding_scope(scope);
    let function_id = if private {
        PRIVATE_STATE_GET_ID
    } else {
        STATE_GET_ID
    };
    let call = || async {
        run_hidden(
            HIDDEN_FAMILY,
            iii.trigger(TriggerRequest {
                function_id: function_id.into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: Some(timeout_ms),
            }),
        )
        .await
        .map_err(|e| HarnessError::State(format!("{function_id} {scope}/{key}: {e}")))
    };
    if private {
        with_private_namespace(iii, timeout_ms, call).await
    } else {
        call().await
    }
}

pub(crate) async fn state_set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: Value,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    if is_deletion_scope(scope) {
        for _ in 0..8 {
            let current = state_get(iii, scope, key, timeout_ms).await?;
            if cas_value(
                iii,
                scope,
                key,
                (!current.is_null()).then_some(current),
                value.clone(),
                timeout_ms,
            )
            .await?
            .is_none()
            {
                return Ok(());
            }
        }
        return Err(HarnessError::State(format!(
            "concurrent lifecycle write {scope}/{key}"
        )));
    }
    run_hidden(
        HIDDEN_FAMILY,
        iii.trigger(TriggerRequest {
            function_id: STATE_SET_ID.into(),
            payload: json!({ "scope": scope, "key": key, "value": value }),
            action: None,
            timeout_ms: Some(timeout_ms),
        }),
    )
    .await
    .map(|_| ())
    .map_err(|e| HarnessError::State(format!("{STATE_SET_ID} {scope}/{key}: {e}")))
}

pub(crate) async fn state_delete(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    if is_deletion_scope(scope) {
        return state_set(iii, scope, key, Value::Null, timeout_ms).await;
    }
    run_hidden(
        HIDDEN_FAMILY,
        iii.trigger(TriggerRequest {
            function_id: STATE_DELETE_ID.into(),
            payload: json!({ "scope": scope, "key": key }),
            action: None,
            timeout_ms: Some(timeout_ms),
        }),
    )
    .await
    .map(|_| ())
    .map_err(|e| HarnessError::State(format!("{STATE_DELETE_ID} {scope}/{key}: {e}")))
}

/// Read the turn record for a session (`None` when absent or null).
pub async fn get_turn(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Option<TurnRecord>, HarnessError> {
    let v = state_get(iii, TURN_SCOPE, session_id, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| HarnessError::State(format!("turn record parse: {e}")))
}

/// Persist the turn record (whole-record write; the loop holds the only
/// writer per session via the per-session lock).
///
/// A timed-out write is retried ONCE: the record is the loop's source of
/// truth and the write is idempotent, so propagating a transient timeout
/// aborts a whole live turn over nothing. The one observed loss
/// (verify-wake-fix-4 postmortem) was an engine-wide ~10s stall that ended
/// moments after the first wait expired — the retry lands where the
/// propagated error killed the turn. Only timeouts retry; every other
/// failure means the write was REJECTED and must surface.
pub async fn put_turn(
    iii: &IIIClient,
    record: &TurnRecord,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(record)
        .map_err(|e| HarnessError::State(format!("turn record serialize: {e}")))?;
    let written = match state_set(
        iii,
        TURN_SCOPE,
        &record.session_id,
        value.clone(),
        timeout_ms,
    )
    .await
    {
        Err(e) if is_timeout(&e) => {
            tracing::warn!(
                session_id = %record.session_id,
                error = %e,
                "turn record persist timed out; retrying once"
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            state_set(iii, TURN_SCOPE, &record.session_id, value, timeout_ms).await
        }
        other => other,
    };
    mark_turn_changed(&record.session_id);
    written
}

/// Whether a state error is the caller-side invocation timeout (the SDK's
/// wording, carried through [`state_set`]'s error mapping). String-matched
/// because `HarnessError::State` flattens the SDK error to text.
fn is_timeout(error: &HarnessError) -> bool {
    error.to_string().contains("timed out")
}

pub async fn delete_turn(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let deleted = state_delete(iii, TURN_SCOPE, session_id, timeout_ms).await;
    mark_turn_changed(session_id);
    deleted
}

/// Turn keys this process wrote or deleted since the orphan redrive's last
/// [`read_changed_turns`] drained them. Marked after the write, even a failed
/// one (a timed-out write may still have landed), so a pass that drains the
/// set reads the key after the write.
static CHANGED_TURNS: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

fn mark_turn_changed(key: &str) {
    CHANGED_TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_owned());
}

/// List every turn record (the pending-call sweep scans these). Keys first,
/// then one `state::get` per key: the scope keeps one record per session ever
/// run, and a single `state::list` reply crossed the engine's 16 MiB frame
/// cap, which wedged the state worker's connection.
// ponytail: sequential gets; bounded concurrency if the sweep gets slow.
pub async fn list_turns(iii: &IIIClient, timeout_ms: u64) -> Result<Vec<TurnRecord>, HarnessError> {
    let keys = parse_keys(&state_list_keys(iii, TURN_SCOPE, timeout_ms).await?);
    let mut records = Vec::with_capacity(keys.len());
    for key in keys {
        records.extend(read_listed_turn(iii, &key, timeout_ms).await?);
    }
    Ok(records)
}

/// One listed key's record. `None` when the key was deleted after it was
/// listed (null, skipped silently) or the record no longer parses (warned),
/// as `parse_list` skips it.
async fn read_listed_turn(
    iii: &IIIClient,
    key: &str,
    timeout_ms: u64,
) -> Result<Option<TurnRecord>, HarnessError> {
    let value = state_get(iii, TURN_SCOPE, key, timeout_ms).await?;
    Ok(serde_json::from_value(value).unwrap_or_else(|e| {
        tracing::warn!(session_id = %key, error = %e, "skipping unparseable turn record");
        None
    }))
}

/// The orphan redrive's read: only what changed since its last pass. Every
/// `state::get` is a root trace carrying the whole record, so re-reading every
/// session ever run each pass flooded the trace store.
///
/// `view` maps each turn key to whether its record was last seen `Running`
/// (an orphan candidate). One `state::list_keys`, then a read of each key
/// that is new to the view (an empty view reads them all: the first pass
/// after boot), that this process wrote or deleted since the last pass, or
/// that was last seen `Running`. Keys gone from `list_keys` leave the view.
/// Returns the records read, which include every `Running` one. Callers
/// serialize passes over one view (a stale pass must not overwrite a newer
/// one's entries).
///
/// Limitation: a second harness process writing an EXISTING key between
/// passes is seen only at the next full pass (the next boot); new keys from
/// other processes are still seen.
pub async fn read_changed_turns(
    iii: &IIIClient,
    view: &mut BTreeMap<String, bool>,
    timeout_ms: u64,
) -> Result<Vec<TurnRecord>, HarnessError> {
    let changed = std::mem::take(&mut *CHANGED_TURNS.lock().unwrap_or_else(|e| e.into_inner()));
    let read = async {
        let keys: BTreeSet<String> =
            parse_keys(&state_list_keys(iii, TURN_SCOPE, timeout_ms).await?)
                .into_iter()
                .collect();
        view.retain(|key, _| keys.contains(key));
        let mut records = Vec::new();
        for key in keys {
            if view.get(&key) == Some(&false) && !changed.contains(&key) {
                continue;
            }
            let record = read_listed_turn(iii, &key, timeout_ms).await?;
            let running = record
                .as_ref()
                .is_some_and(|r| r.status == TurnStatus::Running);
            view.insert(key, running);
            records.extend(record);
        }
        Ok(records)
    }
    .await;
    if read.is_err() {
        // Re-mark what this pass drained: a failed pass loses no write.
        CHANGED_TURNS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(changed);
    }
    read
}

/// Read one trigger binding (`None` when absent or null).
pub async fn get_binding(
    iii: &IIIClient,
    id: &str,
    timeout_ms: u64,
) -> Result<Option<crate::bindings::Binding>, HarnessError> {
    let v = state_get(iii, BINDING_SCOPE, id, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| HarnessError::State(format!("binding parse: {e}")))
}

/// Swap a binding record only if it still holds `expected`. `Ok(None)` means
/// the swap happened; `Ok(Some(current))` means someone else moved it first
/// and `current` is what is there now.
///
/// The claim path needs this: `get` then `put` cannot tell "nobody else fired"
/// from "two fires read the same count and both wrote", which loses a delivery
/// and lets a bounded lifecycle over-spend.
pub async fn cas_binding(
    iii: &IIIClient,
    expected: Option<&crate::bindings::Binding>,
    next: &crate::bindings::Binding,
    timeout_ms: u64,
) -> Result<Option<Value>, HarnessError> {
    let expected_value = match expected {
        Some(b) => Some(
            serde_json::to_value(b)
                .map_err(|e| HarnessError::State(format!("binding serialize: {e}")))?,
        ),
        None => None,
    };
    let value = serde_json::to_value(next)
        .map_err(|e| HarnessError::State(format!("binding serialize: {e}")))?;
    cas_value(
        iii,
        BINDING_SCOPE,
        &next.id,
        expected_value,
        value,
        timeout_ms,
    )
    .await
}

/// Delete a binding only while it still equals the caller's snapshot.
pub async fn cas_delete_binding(
    iii: &IIIClient,
    expected: &crate::bindings::Binding,
    timeout_ms: u64,
) -> Result<bool, HarnessError> {
    let expected_value = serde_json::to_value(expected)
        .map_err(|e| HarnessError::State(format!("binding serialize: {e}")))?;
    Ok(cas_value(
        iii,
        BINDING_SCOPE,
        &expected.id,
        Some(expected_value),
        Value::Null,
        timeout_ms,
    )
    .await?
    .is_none())
}

pub(crate) async fn cas_value(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    expected: Option<Value>,
    value: Value,
    timeout_ms: u64,
) -> Result<Option<Value>, HarnessError> {
    let mut payload = json!({ "scope": scope, "key": key, "value": value });
    if let Some(expected) = expected {
        payload["expected"] = expected;
    }
    let resp = with_private_namespace(iii, timeout_ms, || async {
        run_hidden(
            HIDDEN_FAMILY,
            iii.trigger(TriggerRequest {
                function_id: STATE_CAS_ID.into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(timeout_ms),
            }),
        )
        .await
        .map_err(|e| HarnessError::State(format!("{STATE_CAS_ID} {scope}/{key}: {e}")))
    })
    .await?;
    if resp.get("swapped").and_then(Value::as_bool) == Some(true) {
        Ok(None)
    } else {
        Ok(Some(resp.get("current").cloned().unwrap_or(Value::Null)))
    }
}

/// Every live binding — the owner-gone sweep and the per-owner cap read this.
pub async fn list_bindings(
    iii: &IIIClient,
    timeout_ms: u64,
) -> Result<Vec<crate::bindings::Binding>, HarnessError> {
    Ok(parse_list(
        &state_list(iii, BINDING_SCOPE, timeout_ms).await?,
    ))
}

pub(crate) async fn state_list(
    iii: &IIIClient,
    scope: &str,
    timeout_ms: u64,
) -> Result<Value, HarnessError> {
    let private = is_binding_scope(scope);
    let function_id = if private {
        PRIVATE_STATE_LIST_ID
    } else {
        STATE_LIST_ID
    };
    let call = || async {
        run_hidden(
            HIDDEN_FAMILY,
            iii.trigger(TriggerRequest {
                function_id: function_id.into(),
                payload: json!({ "scope": scope }),
                action: None,
                timeout_ms: Some(timeout_ms),
            }),
        )
        .await
        .map_err(|e| HarnessError::State(format!("{function_id} {scope}: {e}")))
    };
    if private {
        with_private_namespace(iii, timeout_ms, call).await
    } else {
        call().await
    }
}

/// Public scopes only: the state worker has no private `list_keys` accessor.
async fn state_list_keys(
    iii: &IIIClient,
    scope: &str,
    timeout_ms: u64,
) -> Result<Value, HarnessError> {
    run_hidden(
        HIDDEN_FAMILY,
        iii.trigger(TriggerRequest {
            function_id: STATE_LIST_KEYS_ID.into(),
            payload: json!({ "scope": scope }),
            action: None,
            timeout_ms: Some(timeout_ms),
        }),
    )
    .await
    .map_err(|e| HarnessError::State(format!("{STATE_LIST_KEYS_ID} {scope}: {e}")))
}

/// Ask the state worker to reserve our binding scopes and register
/// `harness::state::*`. Idempotent: re-claiming an owned namespace is a
/// no-op, so this is safe to call on every miss.
async fn claim_private_namespace(iii: &IIIClient, timeout_ms: u64) -> Result<(), HarnessError> {
    run_hidden(
        HIDDEN_FAMILY,
        iii.trigger(TriggerRequest {
            function_id: CLAIM_NAMESPACE_ID.into(),
            payload: json!({
                "functions_prefix": NAMESPACE_PREFIX,
                "scopes": [BINDING_SCOPE, BINDING_OWNER_SCOPE, crate::functions::delete_session_tree::OPERATIONS,
                    crate::functions::delete_session_tree::GUARDS, crate::functions::delete_session_tree::DISPATCHES],
            }),
            action: None,
            timeout_ms: Some(timeout_ms),
        }),
    )
    .await
    .map(|_| ())
    .map_err(|e| HarnessError::State(format!("{CLAIM_NAMESPACE_ID}: {e}")))
}

/// The state worker only registers `harness::state::*` once we have claimed
/// our namespace, and a state restart with a volatile adapter forgets the
/// claim. So a MISSING accessor is not fatal: claim (idempotent) and retry
/// once. This single path also covers first boot and a state worker that came
/// up after us — no boot ordering, no configuration.
async fn with_private_namespace<F, Fut>(
    iii: &IIIClient,
    timeout_ms: u64,
    call: F,
) -> Result<Value, HarnessError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<Value, HarnessError>>,
{
    match call().await {
        Err(error)
            if is_unregistered_accessor(&error) || error.to_string().contains("INVALID_SCOPE") =>
        {
            claim_private_namespace(iii, timeout_ms).await?;
            call().await
        }
        outcome => outcome,
    }
}

/// Whether an error says the accessor itself is not registered (as opposed to
/// the state worker being down, which a claim cannot fix).
fn is_unregistered_accessor(error: &HarnessError) -> bool {
    let message = error.to_string();
    message.contains("function_not_found") || message.contains("not found")
}

fn is_binding_scope(scope: &str) -> bool {
    matches!(scope, BINDING_SCOPE | BINDING_OWNER_SCOPE) || is_deletion_scope(scope)
}

fn is_deletion_scope(scope: &str) -> bool {
    matches!(
        scope,
        crate::functions::delete_session_tree::OPERATIONS
            | crate::functions::delete_session_tree::GUARDS
            | crate::functions::delete_session_tree::DISPATCHES
    )
}

/// Tolerate the two `state::list` shapes seen across engines: a bare array of
/// values, or `{ "values": [...] }` / `{ "items": [...] }` / a key→value map.
fn parse_list<T: serde::de::DeserializeOwned>(v: &Value) -> Vec<T> {
    let candidates: Vec<&Value> = match v {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("values").or_else(|| map.get("items")) {
                items.iter().collect()
            } else {
                map.values().collect()
            }
        }
        _ => return Vec::new(),
    };
    candidates
        .into_iter()
        .filter_map(|c| serde_json::from_value::<T>(c.clone()).ok())
        .collect()
}

/// `state::list_keys` replies `{ "keys": [...] }`; a bare array is tolerated too.
fn parse_keys(v: &Value) -> Vec<String> {
    let keys = v.get("keys").unwrap_or(v);
    keys.as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k.as_str().map(str::to_owned))
        .collect()
}

/// One message parked while a step was streaming, waiting for the loop's
/// drain to append it to the transcript (harness.md § Concurrency & steering).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueuedMessage {
    pub id: String,
    pub session_id: String,
    pub message: AgentMessage,
    /// Deterministic transcript entry id the drain appends under, so a
    /// redelivered drain is a no-op.
    pub entry_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Value>,
    pub queued_at: i64,
}

/// Enqueue a message under a fresh unique key — a blind write, safe without
/// the session lock.
pub async fn enqueue_message(
    iii: &IIIClient,
    row: &QueuedMessage,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let key = queue_key(&row.session_id, &row.id);
    let value = serde_json::to_value(row)
        .map_err(|e| HarnessError::State(format!("queued message serialize: {e}")))?;
    state_set(iii, QUEUE_SCOPE, &key, value, timeout_ms).await
}

/// The session's queued messages in arrival order (`queued_at`, then `id`).
// ponytail: state::list scans the whole scope; per-session prefix listing if
// queue volume matters.
pub async fn list_queued(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Vec<QueuedMessage>, HarnessError> {
    let mut rows: Vec<QueuedMessage> = parse_list(&state_list(iii, QUEUE_SCOPE, timeout_ms).await?)
        .into_iter()
        .filter(|r: &QueuedMessage| r.session_id == session_id)
        .collect();
    sort_queued(&mut rows);
    Ok(rows)
}

fn sort_queued(rows: &mut [QueuedMessage]) {
    rows.sort_by(|a, b| (a.queued_at, &a.id).cmp(&(b.queued_at, &b.id)));
}

pub async fn delete_queued(
    iii: &IIIClient,
    session_id: &str,
    id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    state_delete(iii, QUEUE_SCOPE, &queue_key(session_id, id), timeout_ms).await
}

fn queue_key(session_id: &str, id: &str) -> String {
    format!("{session_id}:{id}")
}

pub async fn get_idem(
    iii: &IIIClient,
    key: &str,
    timeout_ms: u64,
) -> Result<Option<IdemRecord>, HarnessError> {
    let v = state_get(iii, IDEM_SCOPE, key, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    serde_json::from_value(v)
        .map(Some)
        .map_err(|e| HarnessError::State(format!("idem record parse: {e}")))
}

pub async fn put_idem(
    iii: &IIIClient,
    key: &str,
    record: &IdemRecord,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(record)
        .map_err(|e| HarnessError::State(format!("idem record serialize: {e}")))?;
    state_set(iii, IDEM_SCOPE, key, value, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_handles_array_and_object_shapes() {
        let rec = json!({
            "turn_id": "t_1", "session_id": "s_1", "status": "running",
            "step": 0, "turn_count": 0, "depth": 0,
            "options": { "model": "m", "max_turns": 16 },
            "created_at": 1, "updated_at": 1
        });
        let as_array = json!([rec]);
        assert_eq!(parse_list::<TurnRecord>(&as_array).len(), 1);
        let as_values = json!({ "values": [rec] });
        assert_eq!(parse_list::<TurnRecord>(&as_values).len(), 1);
        let as_map = json!({ "s_1": rec });
        assert_eq!(parse_list::<TurnRecord>(&as_map).len(), 1);
        assert_eq!(parse_list::<TurnRecord>(&json!(null)).len(), 0);
    }

    #[test]
    fn parse_keys_handles_array_and_object_shapes() {
        assert_eq!(parse_keys(&json!({ "keys": ["a", "b"] })), ["a", "b"]);
        assert_eq!(parse_keys(&json!(["a"])), ["a"]);
        assert!(parse_keys(&json!(null)).is_empty());
    }

    /// The live `harness_turn` scope outgrew the engine's 16 MiB frame cap as
    /// ONE `state::list` reply and wedged the state worker. `list_turns` reads
    /// the keys, then each record; a key deleted in between (null) is skipped
    /// silently and a record that no longer parses is skipped with a warning.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_turns_reads_each_key_never_the_whole_scope() {
        use futures_util::{SinkExt, StreamExt};
        use std::sync::{Arc, Mutex};
        use tokio_tungstenite::tungstenite::Message;

        fn turn(id: &str) -> Value {
            json!({ "turn_id": "t", "session_id": id, "status": "running", "step": 0,
                "turn_count": 0, "depth": 0, "options": { "model": "m", "max_turns": 16 },
                "created_at": 1, "updated_at": 1 })
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = calls.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(msg) = serde_json::from_str::<Value>(frame.to_text().unwrap_or("")) else {
                    continue;
                };
                if msg["type"] != "invokefunction" || msg["invocation_id"].is_null() {
                    continue;
                }
                let function = msg["function_id"].as_str().unwrap_or("").to_owned();
                seen.lock().unwrap().push(function.clone());
                let result = match (function.as_str(), msg["data"]["key"].as_str()) {
                    ("state::list", _) => json!([turn("s_1"), turn("s_2")]),
                    ("state::list_keys", _) => json!({ "keys": ["s_1", "s_gone", "s_bad", "s_2"] }),
                    ("state::get", Some(key @ ("s_1" | "s_2"))) => turn(key),
                    ("state::get", Some("s_bad")) => json!({ "session_id": "s_bad" }),
                    _ => Value::Null,
                };
                let reply = json!({ "type": "invocationresult", "function_id": function,
                    "invocation_id": msg["invocation_id"], "result": result });
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        // Thread-local capture: `list_turns` is polled on this thread.
        #[derive(Clone, Default)]
        struct Logs(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Logs {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let logs = Logs::default();
        let sink = logs.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish();
        let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
        let listed = {
            let _logs = tracing::subscriber::set_default(subscriber);
            list_turns(&iii, 2_000).await
        };
        iii.shutdown();
        server.abort();

        let ids: Vec<String> = listed.unwrap().into_iter().map(|r| r.session_id).collect();
        assert_eq!(ids, ["s_1", "s_2"]);
        // Only the unparseable record warns; the deleted key is skipped silently.
        let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        let warned: Vec<&str> = logs
            .lines()
            .filter(|l| l.contains("skipping unparseable turn record"))
            .collect();
        assert!(
            warned.len() == 1 && warned[0].contains("session_id=s_bad"),
            "{logs}"
        );
        let calls = calls.lock().unwrap();
        assert!(!calls.iter().any(|f| f == "state::list"), "{calls:?}");
        let gets = calls.iter().filter(|f| *f == "state::get").count();
        assert_eq!(gets, 4, "{calls:?}");
    }

    /// The orphan redrive reads only what changed: every key on the first
    /// pass, then keys new to the view, keys this process wrote or deleted, and
    /// keys last seen `Running`. A pass that fails re-marks what it drained.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn read_changed_turns_reads_only_what_changed() {
        use futures_util::{SinkExt, StreamExt};
        use std::sync::{Arc, Mutex};
        use tokio_tungstenite::tungstenite::Message;

        fn turn(id: &str, status: &str) -> Value {
            json!({ "turn_id": "t", "session_id": id, "status": status, "step": 0,
                "turn_count": 0, "depth": 0, "options": { "model": "m", "max_turns": 16 },
                "created_at": 1, "updated_at": 1 })
        }
        fn record(id: &str, status: &str) -> TurnRecord {
            serde_json::from_value(turn(id, status)).unwrap()
        }
        type Store = Arc<Mutex<BTreeMap<String, Value>>>;
        let store: Store = Arc::new(Mutex::new(BTreeMap::from([
            ("rv_a".to_owned(), turn("rv_a", "completed")),
            ("rv_b".to_owned(), turn("rv_b", "failed")),
        ])));
        let calls = Arc::new(Mutex::new(Vec::<String>::new()));
        // A `state::get` of this key is never answered (the read fails).
        let hang = Arc::new(Mutex::new(None::<String>));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (db, seen, stuck) = (store.clone(), calls.clone(), hang.clone());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            while let Some(Ok(frame)) = socket.next().await {
                let Ok(msg) = serde_json::from_str::<Value>(frame.to_text().unwrap_or("")) else {
                    continue;
                };
                if msg["type"] != "invokefunction" || msg["invocation_id"].is_null() {
                    continue;
                }
                let function = msg["function_id"].as_str().unwrap_or("").to_owned();
                let key = msg["data"]["key"].as_str().unwrap_or("").to_owned();
                seen.lock()
                    .unwrap()
                    .push(format!("{function} {key}").trim().to_owned());
                let result = {
                    let mut db = db.lock().unwrap();
                    match function.as_str() {
                        "state::list_keys" => json!({ "keys": db.keys().collect::<Vec<_>>() }),
                        "state::get" if stuck.lock().unwrap().as_ref() == Some(&key) => continue,
                        "state::get" => db.get(&key).cloned().unwrap_or(Value::Null),
                        "state::set" => {
                            db.insert(key, msg["data"]["value"].clone());
                            json!({})
                        }
                        "state::delete" => {
                            db.remove(&key);
                            json!({})
                        }
                        _ => Value::Null,
                    }
                };
                let reply = json!({ "type": "invocationresult", "function_id": function,
                    "invocation_id": msg["invocation_id"], "result": result });
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
        let mut view = BTreeMap::new();
        // One pass; returns the keys it read. Its first call is `list_keys`.
        async fn pass(
            iii: &IIIClient,
            view: &mut BTreeMap<String, bool>,
            calls: &Mutex<Vec<String>>,
        ) -> Vec<String> {
            calls.lock().unwrap().clear();
            read_changed_turns(iii, view, 2_000).await.unwrap();
            let calls = std::mem::take(&mut *calls.lock().unwrap());
            assert_eq!(calls[0], "state::list_keys", "{calls:?}");
            calls[1..]
                .iter()
                .map(|c| c.strip_prefix("state::get ").expect(c).to_owned())
                .collect()
        }

        // First pass after boot: every record.
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a", "rv_b"]);
        // Nothing changed: keys only.
        assert!(pass(&iii, &mut view, &calls).await.is_empty());
        // This process wrote one key: exactly that key.
        put_turn(&iii, &record("rv_a", "running"), 2_000)
            .await
            .unwrap();
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a"]);
        // Last seen Running: re-read every pass.
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a"]);
        // A key new to the view (another process wrote it): read.
        store
            .lock()
            .unwrap()
            .insert("rv_c".into(), turn("rv_c", "completed"));
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a", "rv_c"]);
        // A key gone from list_keys leaves the view.
        store.lock().unwrap().remove("rv_b");
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a"]);
        assert!(!view.contains_key("rv_b"), "{view:?}");
        // A delete by this process marks the key: recreated elsewhere before
        // the pass, it is still read.
        delete_turn(&iii, "rv_c", 2_000).await.unwrap();
        store
            .lock()
            .unwrap()
            .insert("rv_c".into(), turn("rv_c", "running"));
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a", "rv_c"]);
        assert_eq!(view.get("rv_c"), Some(&true));
        put_turn(&iii, &record("rv_a", "completed"), 2_000)
            .await
            .unwrap();
        put_turn(&iii, &record("rv_c", "completed"), 2_000)
            .await
            .unwrap();
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a", "rv_c"]);
        assert!(pass(&iii, &mut view, &calls).await.is_empty());
        // A pass that cannot read a written key re-marks it for the next.
        put_turn(&iii, &record("rv_c", "running"), 2_000)
            .await
            .unwrap();
        *hang.lock().unwrap() = Some("rv_c".into());
        assert!(read_changed_turns(&iii, &mut view, 300).await.is_err());
        *hang.lock().unwrap() = None;
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_c"]);
        assert_eq!(view.get("rv_c"), Some(&true));

        iii.shutdown();
        server.abort();
    }

    /// Only the caller-side invocation timeout retries; rejections (schema,
    /// size, permissions) must surface on the first failure.
    #[test]
    fn only_invocation_timeouts_are_retryable() {
        assert!(is_timeout(&HarnessError::State(
            "state::set harness_turn/s1: invocation timed out".into()
        )));
        assert!(!is_timeout(&HarnessError::State(
            "state::set harness_turn/s1: FORBIDDEN: nope".into()
        )));
        assert!(!is_timeout(&HarnessError::State(
            "turn record serialize: oops".into()
        )));
    }

    #[test]
    fn queued_messages_sort_by_arrival_then_id() {
        fn row(id: &str, at: i64) -> QueuedMessage {
            QueuedMessage {
                id: id.into(),
                session_id: "s_1".into(),
                message: AgentMessage::user_text("hi"),
                entry_id: format!("e_{id}"),
                origin: None,
                queued_at: at,
            }
        }
        let mut rows = vec![row("q_b", 2), row("q_c", 1), row("q_a", 2)];
        sort_queued(&mut rows);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["q_c", "q_a", "q_b"]);
    }
    /// A missing accessor is recoverable (claim + retry); a state worker that
    /// is down, timing out, or erroring is NOT — re-claiming cannot fix it,
    /// and retrying would double every failing call.
    #[test]
    fn only_a_missing_accessor_triggers_a_reclaim() {
        let missing = HarnessError::State(
            "harness::state::get harness_binding/b1: function_not_found: Function \
             harness::state::get not found"
                .into(),
        );
        assert!(is_unregistered_accessor(&missing));

        for fatal in [
            "harness::state::get harness_binding/b1: TIMEOUT: invocation timed out",
            "harness::state::compare-and-set harness_binding/b1: handler error: CAS_ERROR: disk full",
            "harness::state::list harness_binding: connection closed",
        ] {
            assert!(
                !is_unregistered_accessor(&HarnessError::State(fatal.into())),
                "must not re-claim on: {fatal}"
            );
        }
    }

    /// The claim names exactly the scopes the accessors are allowed to serve —
    /// the state worker hard-scopes them, so a drift here would break reads.
    #[test]
    fn the_claim_covers_every_private_scope() {
        for scope in [BINDING_SCOPE, BINDING_OWNER_SCOPE] {
            assert!(
                is_binding_scope(scope),
                "{scope} must route to the accessors"
            );
        }
        assert!(!is_binding_scope(TURN_SCOPE));
        assert!(!is_binding_scope(QUEUE_SCOPE));
        assert!(!is_binding_scope(IDEM_SCOPE));
    }
}
