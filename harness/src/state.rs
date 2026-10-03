//! Durable loop bookkeeping in iii state (harness.md § State).
//!
//! Five scopes: `harness_turn/<session_id>` holds the [`TurnRecord`] (loop
//! progress, per-send options, per-call checkpoints),
//! `harness_prompt/<sha256:hex>` holds each frozen prompt body the records
//! reference ([`PROMPT_SCOPE`]),
//! `harness_idem/<idempotency_key>` holds the webhook-dedupe row,
//! `harness_queue/<session_id>:<id>` holds one [`QueuedMessage`] per message
//! that arrived while a step was streaming (drained by the loop), and
//! `harness_binding/<binding_id>` holds one [`crate::bindings::Binding`].
//! Binding scopes use the state worker's hidden harness API; ordinary
//! bookkeeping keeps the public `state::*` compatibility surface.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

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

/// Read the turn record for a session (`None` when absent or null), with its
/// frozen prompt texts filled back in from `harness_prompt` ([`put_turn`]
/// stores refs only). The record keeps the refs too.
pub async fn get_turn(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Option<TurnRecord>, HarnessError> {
    let v = state_get(iii, TURN_SCOPE, session_id, timeout_ms).await?;
    let Some(mut record) =
        parse_stored_turn(v).map_err(|e| HarnessError::State(format!("turn record parse: {e}")))?
    else {
        return Ok(None);
    };
    hydrate(iii, &mut record, timeout_ms).await?;
    Ok(Some(record))
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
///
/// The frozen prompt texts go to `harness_prompt` by digest and the stored
/// record carries only the refs ([`dehydrate`]); `record` itself is untouched.
/// Bodies are written first, so a stored ref always points at a stored body.
pub async fn put_turn(
    iii: &IIIClient,
    record: &TurnRecord,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let mut stored = record.clone();
    for (digest, text) in dehydrate(&mut stored) {
        store_prompt_body(iii, &digest, &text, timeout_ms).await?;
    }
    let mut value = serde_json::to_value(&stored)
        .map_err(|e| HarnessError::State(format!("turn record serialize: {e}")))?;
    refs_to_stored(&mut value);
    let written =
        set_retrying_timeout(iii, TURN_SCOPE, &record.session_id, value, timeout_ms).await;
    mark_turn_changed(&record.session_id);
    written
}

/// `state::set`, retried ONCE when it times out (see [`put_turn`]).
async fn set_retrying_timeout(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    value: Value,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    match state_set(iii, scope, key, value.clone(), timeout_ms).await {
        Err(e) if is_timeout(&e) => {
            tracing::warn!(scope, key, error = %e, "state write timed out; retrying once");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            state_set(iii, scope, key, value, timeout_ms).await
        }
        other => other,
    }
}

/// Frozen prompt and skill-index bodies, content-addressed: the key is
/// [`prompt_digest`] of the text, the value `{ "body", "created_at" }`. Turn
/// records reference them (`system_prompt_ref`, `baseline_ref`), so one
/// prompt shared by hundreds of sessions is stored once and every record
/// write stays small. Bodies never change; `created_at` is the last write.
pub(crate) const PROMPT_SCOPE: &str = "harness_prompt";

pub(crate) fn prompt_digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{:x}", Sha256::digest(text.as_bytes()))
}

/// Where each ref sits in a stored record: `{"$ref": <digest>}` in its
/// text's own field. A harness from before refs parses that field as an
/// optional string and ignores unknown fields, so a ref under a field of its
/// own would read there as "no prompt": the session would run without its
/// prompt, and that harness's next write would drop the ref for good. An
/// object where it expects a string fails its parse instead.
const STORED_REFS: [(&str, &str, &str); 2] = [
    ("/options", "system_prompt", "system_prompt_ref"),
    ("/options/skill_context", "baseline", "baseline_ref"),
];

fn refs_to_stored(record: &mut Value) {
    for (at, text, slot) in STORED_REFS {
        if let Some(fields) = record.pointer_mut(at).and_then(Value::as_object_mut) {
            if let Some(digest) = fields.remove(slot) {
                fields.insert(text.to_owned(), json!({ "$ref": digest }));
            }
        }
    }
}

/// A stored turn record (`None` for null), its refs moved back to their own
/// fields ([`refs_to_stored`] undone).
fn parse_stored_turn(mut record: Value) -> serde_json::Result<Option<TurnRecord>> {
    for (at, text, slot) in STORED_REFS {
        if let Some(fields) = record.pointer_mut(at).and_then(Value::as_object_mut) {
            if let Some(digest) = fields.get_mut(text).and_then(|t| t.get_mut("$ref")) {
                let digest = digest.take();
                fields.remove(text);
                fields.insert(slot.to_owned(), digest);
            }
        }
    }
    serde_json::from_value(record)
}

/// The record's frozen texts, each with its `harness_prompt` ref.
fn prompt_fields(record: &mut TurnRecord) -> Vec<(&mut Option<String>, &mut Option<String>)> {
    let options = &mut record.options;
    let mut fields = vec![(&mut options.system_prompt, &mut options.system_prompt_ref)];
    if let Some(context) = options.skill_context.as_mut() {
        fields.push((&mut context.baseline, &mut context.baseline_ref));
    }
    fields
}

/// Move each frozen text present in `record` to its ref; returns the
/// `(digest, text)` bodies to store. A ref without its text (a listed record)
/// stays as it is.
pub(crate) fn dehydrate(record: &mut TurnRecord) -> Vec<(String, String)> {
    prompt_fields(record)
        .into_iter()
        .filter_map(|(text, slot)| {
            let text = text.take()?;
            let digest = prompt_digest(&text);
            *slot = Some(digest.clone());
            Some((digest, text))
        })
        .collect()
}

/// Fill each frozen text that is absent but referenced. A ref whose body is
/// gone is an error: an empty prompt would re-resolve and change the session.
async fn hydrate(
    iii: &IIIClient,
    record: &mut TurnRecord,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let session_id = record.session_id.clone();
    for (text, slot) in prompt_fields(record) {
        if let (None, Some(digest)) = (&*text, &*slot) {
            *text = Some(load_prompt_body(iii, digest, &session_id, timeout_ms).await?);
        }
    }
    Ok(())
}

/// How long this process trusts, without asking the store, that a body it
/// wrote is still there; its next write after that stores the body again.
/// Short on purpose. The store can lose a body behind this process (a reset,
/// or a crash between its scope files: the state worker flushes scopes in no
/// fixed order), and [`BODY_CACHE`] hides the loss from this process's reads
/// until a restart. Writing the body again within minutes mends every record
/// that references it. Cost: one body write per prompt in use per window.
///
/// It also keeps the `created_at` of a body a new record starts referencing
/// within minutes of now, far under the collector's grace. The collector must
/// still not delete a body whose `created_at` moved after it read it: a
/// rewrite can land between its read and its delete.
const KNOWN_BODY_TTL_MS: i64 = 5 * 60 * 1000;

/// Digests this process wrote, with when.
static KNOWN_BODIES: std::sync::Mutex<BTreeMap<String, i64>> =
    std::sync::Mutex::new(BTreeMap::new());

async fn store_prompt_body(
    iii: &IIIClient,
    digest: &str,
    text: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let now = AgentMessage::now_ms();
    let fresh = |known: &BTreeMap<String, i64>| {
        known
            .get(digest)
            .is_some_and(|at| now - at < KNOWN_BODY_TTL_MS)
    };
    if fresh(&KNOWN_BODIES.lock().unwrap_or_else(|e| e.into_inner())) {
        return Ok(());
    }
    let body = json!({ "body": text, "created_at": now });
    set_retrying_timeout(iii, PROMPT_SCOPE, digest, body, timeout_ms).await?;
    let mut known = KNOWN_BODIES.lock().unwrap_or_else(|e| e.into_inner());
    known.retain(|_, at| now - *at < KNOWN_BODY_TTL_MS);
    known.insert(digest.to_owned(), now);
    Ok(())
}

/// Bodies read back. They never change, so an entry never goes stale.
// ponytail: FIFO, not LRU; the live stack has ~40 distinct bodies.
static BODY_CACHE: std::sync::Mutex<VecDeque<(String, String)>> =
    std::sync::Mutex::new(VecDeque::new());
const BODY_CACHE_CAP: usize = 256;

async fn load_prompt_body(
    iii: &IIIClient,
    digest: &str,
    session_id: &str,
    timeout_ms: u64,
) -> Result<String, HarnessError> {
    let cached = BODY_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(d, _)| d == digest)
        .map(|(_, body)| body.clone());
    if let Some(body) = cached {
        return Ok(body);
    }
    let value = state_get(iii, PROMPT_SCOPE, digest, timeout_ms).await?;
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HarnessError::State(format!("missing prompt body {digest} for {session_id}"))
        })?
        .to_owned();
    let mut cache = BODY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= BODY_CACHE_CAP {
        cache.pop_front();
    }
    cache.push_back((digest.to_owned(), body.clone()));
    Ok(body)
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
/// cap, which wedged the state worker's connection. Records come back as
/// stored (prompt refs, no prompt text): read the text through [`get_turn`].
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
    Ok(parse_stored_turn(value).unwrap_or_else(|e| {
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
/// Limitation: an EXISTING key rewritten behind this process's writes (by a
/// second harness process, a state-store rollback, a console edit) is seen
/// only at the next full read: the pending sweep's
/// ([`crate::inflight::read_all_turns`], daily by default) or the first pass
/// after boot. New keys from other processes are still seen.
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
    /// keys last seen `Running`. A pass that fails re-marks what it drained,
    /// and a write is marked after it lands.
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
        // A pass starts while this key's write is in flight: the set is
        // drained before the write lands.
        let drain = Arc::new(Mutex::new(None::<String>));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (db, seen, stuck, midway) = (store.clone(), calls.clone(), hang.clone(), drain.clone());
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
                            if midway.lock().unwrap().take().is_some_and(|k| k == key) {
                                CHANGED_TURNS.lock().unwrap().clear();
                            }
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
        // A pass that drained the set while a write was in flight (it read the
        // old record) loses nothing: the key is marked after the write lands.
        *drain.lock().unwrap() = Some("rv_a".into());
        put_turn(&iii, &record("rv_a", "failed"), 2_000)
            .await
            .unwrap();
        assert_eq!(pass(&iii, &mut view, &calls).await, ["rv_a"]);
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

    type Store = std::sync::Arc<std::sync::Mutex<BTreeMap<(String, String), Value>>>;
    type Calls = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// A prompt body the fake engine refuses to store.
    const REJECTED_BODY: &str = "fake_state rejects this body";

    /// A store-backed fake engine serving `state::{get,set,delete,list_keys}`
    /// over every scope. Logs each call as `"<function> <scope>/<key>"`. A
    /// `state::set` of [`REJECTED_BODY`] fails.
    async fn fake_state() -> (IIIClient, Store, Calls, tokio::task::JoinHandle<()>) {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (store, calls) = (Store::default(), Calls::default());
        let (db, seen) = (store.clone(), calls.clone());
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
                let scope = msg["data"]["scope"].as_str().unwrap_or("").to_owned();
                let key = msg["data"]["key"].as_str().unwrap_or("").to_owned();
                seen.lock()
                    .unwrap()
                    .push(format!("{function} {scope}/{key}"));
                let rejected =
                    function == "state::set" && msg["data"]["value"]["body"] == REJECTED_BODY;
                let result = {
                    let mut db = db.lock().unwrap();
                    let at = (scope.clone(), key);
                    match function.as_str() {
                        "state::list_keys" => json!({ "keys": db.keys()
                            .filter(|(s, _)| *s == scope).map(|(_, k)| k).collect::<Vec<_>>() }),
                        "state::get" => db.get(&at).cloned().unwrap_or(Value::Null),
                        "state::set" if rejected => Value::Null,
                        "state::set" => {
                            db.insert(at, msg["data"]["value"].clone());
                            json!({})
                        }
                        "state::delete" => {
                            db.remove(&at);
                            json!({})
                        }
                        _ => Value::Null,
                    }
                };
                let mut reply = json!({ "type": "invocationresult", "function_id": function,
                    "invocation_id": msg["invocation_id"] });
                if rejected {
                    reply["error"] = json!({ "code": "rejected", "message": "rejected" });
                } else {
                    reply["result"] = result;
                }
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
        (iii, store, calls, server)
    }

    fn prompt_record(session_id: &str, prompt: Option<&str>, baseline: Option<&str>) -> TurnRecord {
        let mut record: TurnRecord = serde_json::from_value(json!({
            "turn_id": "t", "session_id": session_id, "status": "completed", "step": 0,
            "turn_count": 0, "depth": 0, "options": { "model": "m", "max_turns": 16 },
            "created_at": 1, "updated_at": 1 }))
        .unwrap();
        record.options.system_prompt = prompt.map(str::to_owned);
        record.options.skill_context = baseline.map(|b| crate::types::turn::SkillContext {
            filter: None,
            baseline: Some(b.to_owned()),
            baseline_ref: None,
        });
        record
    }

    fn stored(store: &Store, scope: &str, key: &str) -> Value {
        let db = store.lock().unwrap();
        db.get(&(scope.to_owned(), key.to_owned()))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn body_keys(store: &Store) -> Vec<String> {
        let db = store.lock().unwrap();
        db.keys()
            .filter(|(s, _)| s == PROMPT_SCOPE)
            .map(|(_, k)| k.clone())
            .collect()
    }

    fn body_writes(calls: &Calls) -> usize {
        let prefix = format!("state::set {PROMPT_SCOPE}/");
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with(&prefix))
            .count()
    }

    // Prompt texts are unique per test: the known-body set is process-wide,
    // and each test has its own fake store.

    /// The stored record carries refs only; the bodies live once in
    /// `harness_prompt`; `get_turn` restores the text (and keeps the refs).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn put_then_get_restores_the_prompt_and_stores_only_a_ref() {
        let (iii, store, _calls, server) = fake_state().await;
        let prompt = "put_then_get frozen prompt. ".repeat(1_000);
        let index = "<available_skills>put_then_get</available_skills>";
        let record = prompt_record("pg_1", Some(&prompt), Some(index));
        put_turn(&iii, &record, 2_000).await.unwrap();

        let value = stored(&store, TURN_SCOPE, "pg_1");
        let options = &value["options"];
        assert_eq!(
            options["system_prompt"],
            json!({ "$ref": prompt_digest(&prompt) })
        );
        assert!(options.get("system_prompt_ref").is_none(), "{options}");
        assert_eq!(
            options["skill_context"]["baseline"],
            json!({ "$ref": prompt_digest(index) })
        );
        assert!(
            options["skill_context"].get("baseline_ref").is_none(),
            "{options}"
        );
        assert_eq!(body_keys(&store).len(), 2);
        let body = stored(&store, PROMPT_SCOPE, &prompt_digest(&prompt));
        assert_eq!(body["body"], prompt.as_str());
        assert!(body["created_at"].as_i64().is_some(), "{body}");

        let read = get_turn(&iii, "pg_1", 2_000).await.unwrap().unwrap();
        let mut expected = record.clone();
        expected.options.system_prompt_ref = Some(prompt_digest(&prompt));
        expected
            .options
            .skill_context
            .as_mut()
            .unwrap()
            .baseline_ref = Some(prompt_digest(index));
        assert_eq!(read, expected);
        iii.shutdown();
        server.abort();
    }

    /// Two sessions on one prompt share one body. A process writes a body once
    /// (it remembers what it stored); concurrent first writes set the same key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_prompt_two_sessions_one_body() {
        let (iii, store, calls, server) = fake_state().await;
        let prompt = "same_prompt_two_sessions shared prompt";
        put_turn(&iii, &prompt_record("sp_1", Some(prompt), None), 2_000)
            .await
            .unwrap();
        put_turn(&iii, &prompt_record("sp_2", Some(prompt), None), 2_000)
            .await
            .unwrap();
        assert_eq!(body_keys(&store), [prompt_digest(prompt)]);
        assert_eq!(body_writes(&calls), 1);
        // Known for longer than the TTL: written again (the collector may
        // have deleted it since).
        *KNOWN_BODIES
            .lock()
            .unwrap()
            .get_mut(&prompt_digest(prompt))
            .unwrap() -= KNOWN_BODY_TTL_MS;
        put_turn(&iii, &prompt_record("sp_2", Some(prompt), None), 2_000)
            .await
            .unwrap();
        assert_eq!(body_writes(&calls), 2);

        let racing = "same_prompt_two_sessions racing prompt";
        let (a, b) = (
            prompt_record("sp_3", Some(racing), None),
            prompt_record("sp_4", Some(racing), None),
        );
        let (ra, rb) = tokio::join!(put_turn(&iii, &a, 2_000), put_turn(&iii, &b, 2_000));
        ra.unwrap();
        rb.unwrap();
        assert_eq!(body_keys(&store).len(), 2);
        for sid in ["sp_1", "sp_2", "sp_3", "sp_4"] {
            let read = get_turn(&iii, sid, 2_000).await.unwrap().unwrap();
            let want = if sid < "sp_3" { prompt } else { racing };
            assert_eq!(read.options.system_prompt.as_deref(), Some(want), "{sid}");
        }
        iii.shutdown();
        server.abort();
    }

    /// Records written before refs existed (inline text, no ref) read as
    /// before, without touching `harness_prompt`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn inline_record_still_reads() {
        let (iii, store, calls, server) = fake_state().await;
        let record = prompt_record("ir_1", Some("inline_record prompt"), Some("inline index"));
        store.lock().unwrap().insert(
            (TURN_SCOPE.into(), "ir_1".into()),
            serde_json::to_value(&record).unwrap(),
        );
        let read = get_turn(&iii, "ir_1", 2_000).await.unwrap().unwrap();
        assert_eq!(read, record);
        assert_eq!(
            *calls.lock().unwrap(),
            [format!("state::get {TURN_SCOPE}/ir_1")]
        );
        iii.shutdown();
        server.abort();
    }

    /// A listed record is dehydrated (refs, no text). Re-putting it keeps the
    /// refs and writes no body; `get_turn` still restores the text.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dehydrated_record_reput_keeps_ref() {
        let (iii, store, calls, server) = fake_state().await;
        let (prompt, index) = ("dehydrated_reput prompt", "dehydrated_reput index");
        put_turn(
            &iii,
            &prompt_record("dr_1", Some(prompt), Some(index)),
            2_000,
        )
        .await
        .unwrap();
        let listed = list_turns(&iii, 2_000).await.unwrap().remove(0);
        assert_eq!(listed.options.system_prompt, None);
        assert_eq!(
            listed.options.system_prompt_ref,
            Some(prompt_digest(prompt))
        );
        let context = listed.options.skill_context.clone().unwrap();
        assert_eq!(context.baseline, None);
        assert_eq!(context.baseline_ref, Some(prompt_digest(index)));

        let writes = body_writes(&calls);
        put_turn(&iii, &listed, 2_000).await.unwrap();
        assert_eq!(
            body_writes(&calls),
            writes,
            "a dehydrated re-put writes no body"
        );
        let value = stored(&store, TURN_SCOPE, "dr_1");
        assert_eq!(
            value["options"]["system_prompt"]["$ref"],
            prompt_digest(prompt)
        );
        assert_eq!(
            value["options"]["skill_context"]["baseline"]["$ref"],
            prompt_digest(index)
        );
        let read = get_turn(&iii, "dr_1", 2_000).await.unwrap().unwrap();
        assert_eq!(read.options.system_prompt.as_deref(), Some(prompt));
        assert_eq!(
            read.options.skill_context.unwrap().baseline.as_deref(),
            Some(index)
        );
        iii.shutdown();
        server.abort();
    }

    /// A ref whose body is gone is a hard error, never an empty prompt (which
    /// would re-resolve the prompt and change the session). The record holds
    /// the ref as its own field, as the first build with refs stored it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missing_body_is_an_error() {
        let (iii, store, _calls, server) = fake_state().await;
        let mut record = prompt_record("mb_1", None, None);
        record.options.system_prompt_ref = Some("sha256:missing_body_is_an_error".into());
        store.lock().unwrap().insert(
            (TURN_SCOPE.into(), "mb_1".into()),
            serde_json::to_value(&record).unwrap(),
        );
        let error = get_turn(&iii, "mb_1", 2_000).await.unwrap_err().to_string();
        assert!(
            error.contains("missing prompt body sha256:missing_body_is_an_error for mb_1"),
            "{error}"
        );
        iii.shutdown();
        server.abort();
    }

    /// A harness from before refs parses `system_prompt` and `baseline` as
    /// optional strings and ignores unknown fields. A stored ref must fail
    /// that parse: read as "no prompt", the session would run without its
    /// prompt, and that harness's next write would drop the ref for good.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pre_ref_harness_cannot_read_a_ref_as_no_prompt() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct PreRefOptions {
            system_prompt: Option<String>,
            skill_context: Option<PreRefContext>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct PreRefContext {
            baseline: Option<String>,
        }

        let (iii, store, _calls, server) = fake_state().await;
        let (prompt, index) = ("pre_ref prompt", "pre_ref index");
        put_turn(&iii, &prompt_record("pr_1", Some(prompt), None), 2_000)
            .await
            .unwrap();
        put_turn(&iii, &prompt_record("pr_2", None, Some(index)), 2_000)
            .await
            .unwrap();
        for sid in ["pr_1", "pr_2"] {
            let options = stored(&store, TURN_SCOPE, sid)["options"].clone();
            assert!(
                serde_json::from_value::<PreRefOptions>(options.clone()).is_err(),
                "{options}"
            );
        }
        let first = get_turn(&iii, "pr_1", 2_000).await.unwrap().unwrap();
        assert_eq!(first.options.system_prompt.as_deref(), Some(prompt));
        let second = get_turn(&iii, "pr_2", 2_000).await.unwrap().unwrap();
        let context = second.options.skill_context.unwrap();
        assert_eq!(context.baseline.as_deref(), Some(index));
        iii.shutdown();
        server.abort();
    }

    /// The store can lose a body this process wrote (a reset, or a crash
    /// between its scope files). The process trusts its own write for minutes
    /// only: the next write after that stores the body again, which mends the
    /// records already referencing it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_lost_body_is_stored_again_minutes_later() {
        let (iii, store, _calls, server) = fake_state().await;
        let prompt = "lost_body prompt";
        let digest = prompt_digest(prompt);
        put_turn(&iii, &prompt_record("lb_1", Some(prompt), None), 2_000)
            .await
            .unwrap();
        store
            .lock()
            .unwrap()
            .remove(&(PROMPT_SCOPE.to_owned(), digest.clone()));
        *KNOWN_BODIES.lock().unwrap().get_mut(&digest).unwrap() -= 10 * 60 * 1000;
        put_turn(&iii, &prompt_record("lb_2", Some(prompt), None), 2_000)
            .await
            .unwrap();
        assert_eq!(body_keys(&store), [digest]);
        let read = get_turn(&iii, "lb_1", 2_000).await.unwrap().unwrap();
        assert_eq!(read.options.system_prompt.as_deref(), Some(prompt));
        iii.shutdown();
        server.abort();
    }

    /// A stored ref always points at a stored body: the bodies are written
    /// before the record, and a failed body write leaves the record as it was.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bodies_are_written_before_the_record_or_not_at_all() {
        let (iii, store, calls, server) = fake_state().await;
        let (prompt, index) = ("body_order prompt", "body_order index");
        put_turn(
            &iii,
            &prompt_record("bo_1", Some(prompt), Some(index)),
            2_000,
        )
        .await
        .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            [
                format!("state::set {PROMPT_SCOPE}/{}", prompt_digest(prompt)),
                format!("state::set {PROMPT_SCOPE}/{}", prompt_digest(index)),
                format!("state::set {TURN_SCOPE}/bo_1"),
            ]
        );

        let before = stored(&store, TURN_SCOPE, "bo_1");
        let error = put_turn(
            &iii,
            &prompt_record("bo_1", Some(REJECTED_BODY), None),
            2_000,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("rejected"), "{error}");
        assert_eq!(
            calls.lock().unwrap().last(),
            Some(&format!(
                "state::set {PROMPT_SCOPE}/{}",
                prompt_digest(REJECTED_BODY)
            ))
        );
        assert_eq!(stored(&store, TURN_SCOPE, "bo_1"), before);
        iii.shutdown();
        server.abort();
    }
}
