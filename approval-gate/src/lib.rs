//! Approval gate. Subscribes to `agent::before_function_call` and blocks calls
//! whose `function_call.function_id` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

pub mod config;
pub mod manifest;

pub use config::{InterceptorRule, WorkerConfig};

use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};

pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const FN_LIST_UNDELIVERED: &str = "approval::list_undelivered";
pub const FN_ACK_DELIVERED: &str = "approval::ack_delivered";
pub const FN_SWEEP_SESSION: &str = "approval::sweep_session";
pub const FN_LOOKUP_RECORD: &str = "approval::lookup_record";
/// Default `approval_state_scope` (matches [`WorkerConfig::default`]).
pub const STATE_SCOPE: &str = "approvals";

fn rule_for<'a>(rules: &'a [InterceptorRule], function_id: &str) -> Option<&'a InterceptorRule> {
    rules.iter().find(|r| r.function_id == function_id)
}

fn merge_from_approval_marker_if_needed(
    inject: bool,
    args: Value,
    function_call_id: &str,
    session_id: &str,
) -> Value {
    if !inject {
        return args;
    }
    let marker = json!({
        "call_id": function_call_id,
        "session_id": session_id,
    });
    match args {
        Value::Object(mut m) => {
            m.insert("__from_approval".into(), marker);
            Value::Object(m)
        }
        other if other.is_null() => json!({ "__from_approval": marker }),
        other => json!({
            "payload": other,
            "__from_approval": marker,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClassifierDecision {
    Auto,
    Deny { reason: String },
    Ask,
}

/// Parse classifier JSON (`decision` tag: auto | deny | ask).
pub(crate) fn interpret_classifier_reply(value: &Value) -> Result<ClassifierDecision, ()> {
    let tag = value.get("decision").and_then(Value::as_str).ok_or(())?;
    match tag {
        "auto" => Ok(ClassifierDecision::Auto),
        "deny" => Ok(ClassifierDecision::Deny {
            reason: value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("denied")
                .to_string(),
        }),
        "ask" => Ok(ClassifierDecision::Ask),
        _ => Err(()),
    }
}

/// True if `status` is one of the terminal states a stitched system message
/// should be built from. `pending` and `approved` are intermediate.
pub fn is_terminal_status(status: &str) -> bool {
    matches!(status, "executed" | "failed" | "denied" | "timed_out")
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingCall {
    pub session_id: String,
    pub function_call_id: String,
    pub function_id: String,
    pub args: Value,
    pub approval_required: Vec<String>,
    pub event_id: String,
    pub reply_stream: String,
}

impl IncomingCall {
    pub fn requires_approval(&self) -> bool {
        self.approval_required
            .iter()
            .any(|n| n == &self.function_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

/// Wire-format decision string used by `approval::resolve` and stored
/// as the `status` field of resolved approval records.
///
/// Serializes / deserializes as `"allow"` or `"deny"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireDecision {
    Allow,
    Deny,
}

/// Build the state-store key for a pending approval entry.
///
/// `session_id` and `function_call_id` must not contain `/`. They are caller-controlled
/// IDs minted by turn-orchestrator; today neither format uses the separator.
pub fn pending_key(session_id: &str, function_call_id: &str) -> String {
    debug_assert!(!session_id.contains('/'), "session_id must not contain '/'");
    debug_assert!(
        !function_call_id.contains('/'),
        "function_call_id must not contain '/'"
    );
    format!("{session_id}/{function_call_id}")
}

pub fn extract_call(envelope: &Value) -> Option<IncomingCall> {
    let event_id = envelope
        .get("event_id")
        .and_then(Value::as_str)?
        .to_string();
    let reply_stream = envelope
        .get("reply_stream")
        .and_then(Value::as_str)?
        .to_string();
    let inner = envelope.get("payload").unwrap_or(envelope);
    let session_id = inner.get("session_id").and_then(Value::as_str)?.to_string();
    let fc = inner
        .get("function_call")
        .or_else(|| inner.get("tool_call"))?;
    let function_id = fc
        .get("function_id")
        .or_else(|| fc.get("name"))
        .and_then(Value::as_str)?
        .to_string();
    Some(IncomingCall {
        session_id,
        function_call_id: fc.get("id").and_then(Value::as_str)?.to_string(),
        function_id,
        args: fc.get("arguments").cloned().unwrap_or_else(|| json!({})),
        approval_required: inner
            .get("approval_required")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        event_id,
        reply_stream,
    })
}

pub fn build_pending_record(
    function_call_id: &str,
    function_id: &str,
    args: &Value,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    json!({
        "function_call_id": function_call_id,
        "function_id": function_id,
        "args": args,
        "status": "pending",
        "expires_at": now_ms.saturating_add(timeout_ms),
    })
}

/// Build a new record by transitioning a pending base record to a terminal
/// status. All terminal fields (`result`, `error`, `decision_reason`) are
/// optional; only the ones provided are attached. Existing fields on the
/// base (including `delivered_in_turn_id` and `resolved_at` if present) are
/// preserved. The first transition into a terminal status stamps
/// `resolved_at`.
pub fn transition_record(
    base: &Value,
    new_status: &str,
    result: Option<Value>,
    error: Option<String>,
    decision_reason: Option<String>,
) -> Value {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    transition_record_with_now(base, new_status, result, error, decision_reason, now_ms)
}

/// Testable variant of [`transition_record`] that takes `now_ms` directly.
pub fn transition_record_with_now(
    base: &Value,
    new_status: &str,
    result: Option<Value>,
    error: Option<String>,
    decision_reason: Option<String>,
    now_ms: u64,
) -> Value {
    let mut rec = base.clone();
    if let Some(obj) = rec.as_object_mut() {
        obj.insert("status".into(), Value::String(new_status.to_string()));
        if let Some(r) = result {
            obj.insert("result".into(), r);
        }
        if let Some(e) = error {
            obj.insert("error".into(), Value::String(e));
        }
        if let Some(reason) = decision_reason {
            obj.insert("decision_reason".into(), Value::String(reason));
        }
        if is_terminal_status(new_status) && !obj.contains_key("resolved_at") {
            obj.insert("resolved_at".into(), Value::Number(now_ms.into()));
        }
    }
    rec
}

pub fn block_reply_for(decision: &Decision) -> Value {
    match decision {
        Decision::Allow => json!({ "block": false }),
        Decision::Deny { reason } => json!({
            "block": true,
            "reason": format!("approval-gate: {reason}"),
        }),
    }
}

pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub list_undelivered: FunctionRef,
    pub ack_delivered: FunctionRef,
    pub sweep_session: FunctionRef,
    pub lookup_record: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
}

#[async_trait::async_trait]
pub trait StateBus: Send + Sync {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError>;
    async fn get(&self, scope: &str, key: &str) -> Option<Value>;
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value>;
}

/// Invokes an iii function with arguments and returns its result or an error
/// string. Abstracted so tests can stub the underlying call.
#[async_trait::async_trait]
pub trait FunctionExecutor: Send + Sync {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        function_call_id: &str,
        session_id: &str,
    ) -> Result<Value, String>;
}

/// Production [`FunctionExecutor`] backed by `iii.trigger`.
pub struct IiiFunctionExecutor {
    pub iii: III,
    pub rules: Arc<Vec<InterceptorRule>>,
}

#[async_trait::async_trait]
impl FunctionExecutor for IiiFunctionExecutor {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        function_call_id: &str,
        session_id: &str,
    ) -> Result<Value, String> {
        let inject =
            rule_for(self.rules.as_slice(), function_id).is_some_and(|r| r.inject_approval_marker);
        let payload =
            merge_from_approval_marker_if_needed(inject, args, function_call_id, session_id);
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: None,
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// Decide whether a call is gated; if so, write a pending record and return
/// the structured pending hook reply. If not gated, return `{block: false}`
/// and do nothing.
pub async fn handle_intercept(
    bus: &dyn StateBus,
    state_scope: &str,
    call: &IncomingCall,
    now_ms: u64,
    timeout_ms: u64,
    force_pending: bool,
) -> Value {
    if !force_pending && !call.requires_approval() {
        return json!({ "block": false });
    }
    let record = build_pending_record(
        &call.function_call_id,
        &call.function_id,
        &call.args,
        now_ms,
        timeout_ms,
    );
    if let Err(err) = bus
        .set(
            state_scope,
            &pending_key(&call.session_id, &call.function_call_id),
            record,
        )
        .await
    {
        tracing::error!(
            "approval-gate: failed to write pending record for {}/{}: {err}",
            call.session_id,
            call.function_call_id
        );
        // Fail open: better to let the call proceed than to silently drop it.
        return json!({ "block": false });
    }
    json!({
        "block": true,
        "reason": "approval-gate: pending_approval",
        "status": "pending",
        "call_id": call.function_call_id,
        "function_id": call.function_id,
    })
}

/// Lookup a single approval record by session + call id (for shell bypass validation).
pub async fn handle_lookup_record(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let function_call_id = payload
        .get("function_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || function_call_id.is_empty() {
        return Value::Null;
    }
    let key = pending_key(session_id, function_call_id);
    bus.get(state_scope, &key).await.unwrap_or(Value::Null)
}

/// Return Some(timed_out_record) if `rec` is pending and `now_ms` is past
/// `expires_at`; otherwise None. Pure function — does not write state.
pub fn maybe_flip_timed_out(rec: &Value, now_ms: u64) -> Option<Value> {
    if rec.get("status").and_then(Value::as_str) != Some("pending") {
        return None;
    }
    let exp = rec.get("expires_at").and_then(Value::as_u64)?;
    if now_ms < exp {
        return None;
    }
    Some(transition_record(
        rec,
        "timed_out",
        None,
        None,
        Some("timeout".into()),
    ))
}

/// Map a legacy approval record (pre-trigger-model) to the new shape.
/// Returns `None` if the record is already in the new shape.
pub fn migrate_legacy_record(rec: &Value) -> Option<Value> {
    let status = rec.get("status").and_then(Value::as_str)?;
    let (new_status, reason_to_carry) = match status {
        "allow" => ("executed", None),
        "deny" => (
            "denied",
            rec.get("reason")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        _ => return None,
    };
    let mut migrated = transition_record(rec, new_status, None, None, reason_to_carry);
    migrated
        .as_object_mut()
        .unwrap()
        .insert("legacy_migrated".into(), Value::Bool(true));
    Some(migrated)
}

pub async fn handle_resolve(
    bus: &dyn StateBus,
    exec: &dyn FunctionExecutor,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let function_call_id = payload
        .get("function_call_id")
        .or_else(|| payload.get("tool_call_id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || function_call_id.is_empty() {
        return json!({ "ok": false, "error": "missing_id" });
    }
    let decision: WireDecision = match payload.get("decision").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(d) => d,
            Err(_) => return json!({ "ok": false, "error": "bad_decision" }),
        },
        None => return json!({ "ok": false, "error": "bad_decision" }),
    };
    let key = pending_key(session_id, function_call_id);
    let Some(existing) = bus.get(state_scope, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };

    // Lazy timeout flip (covered by Task 7 tests).
    let existing = match maybe_flip_timed_out(&existing, now_ms) {
        Some(flipped) => {
            let _ = bus.set(state_scope, &key, flipped.clone()).await;
            return json!({ "ok": false, "error": "timed_out" });
        }
        None => existing,
    };

    if existing.get("status").and_then(Value::as_str) != Some("pending") {
        return json!({ "ok": false, "error": "already_resolved" });
    }

    match decision {
        WireDecision::Deny => {
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            let denied = transition_record(&existing, "denied", None, None, Some(reason));
            if let Err(e) = bus.set(state_scope, &key, denied).await {
                tracing::error!("approval-gate: failed to write denied record: {e}");
                return json!({ "ok": false, "error": "state_write_failed" });
            }
            json!({ "ok": true })
        }
        WireDecision::Allow => {
            let function_id = existing
                .get("function_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let args = existing.get("args").cloned().unwrap_or(json!({}));
            let approved = transition_record(&existing, "approved", None, None, None);
            // Best-effort intermediate write; if it fails, still try to invoke.
            let _ = bus.set(state_scope, &key, approved.clone()).await;
            match exec
                .invoke(&function_id, args, function_call_id, session_id)
                .await
            {
                Ok(result) => {
                    let executed =
                        transition_record(&approved, "executed", Some(result), None, None);
                    if let Err(e) = bus.set(state_scope, &key, executed).await {
                        tracing::error!("approval-gate: failed to write executed record: {e}");
                        return json!({ "ok": false, "error": "state_write_failed" });
                    }
                }
                Err(error) => {
                    let failed = transition_record(&approved, "failed", None, Some(error), None);
                    if let Err(e) = bus.set(state_scope, &key, failed).await {
                        tracing::error!("approval-gate: failed to write failed record: {e}");
                        return json!({ "ok": false, "error": "state_write_failed" });
                    }
                }
            }
            json!({ "ok": true })
        }
    }
}

pub async fn handle_list_pending(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "pending": [] });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let pending: Vec<Value> = all
        .into_iter()
        .filter(|v| {
            if migrate_legacy_record(v).is_some() {
                return false;
            }
            v.get("status").and_then(Value::as_str) == Some("pending")
        })
        .collect();
    json!({ "pending": pending })
}

/// Default cap for `handle_list_undelivered` responses. A single LLM turn
/// should never be asked to ingest more than this many stitched approval
/// messages; older entries beyond the cap stay unacked and are reported via
/// the `omitted` counter so the caller can render a summary line.
pub const LIST_UNDELIVERED_DEFAULT_LIMIT: usize = 50;

/// Return terminal-status records for a session that haven't been stamped
/// with `delivered_in_turn_id`. Lazy timeout: pending records past
/// `expires_at` (as observed at `now_ms`) are flipped to `timed_out` before
/// the filter so they surface here in the same call.
///
/// Sorted oldest-first by `resolved_at` (records missing `resolved_at` sort
/// last as `u64::MAX`). Capped at `limit` (default
/// [`LIST_UNDELIVERED_DEFAULT_LIMIT`]); the response always includes an
/// `omitted` field counting entries left behind.
pub async fn handle_list_undelivered(
    bus: &dyn StateBus,
    state_scope: &str,
    payload: Value,
    now_ms: u64,
) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "entries": [], "omitted": 0 });
    }
    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(LIST_UNDELIVERED_DEFAULT_LIMIT);
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut entries: Vec<Value> = Vec::new();
    for rec in all {
        // Defensive scope: some bus backends ignore the prefix and return
        // every record in `state_scope`. Filter by stamped `session_id`:
        //
        //   - record has session_id matching ours → keep
        //   - record has session_id different from ours → drop
        //   - record lacks session_id AND is in "allow"/"deny" pre-trigger
        //     legacy form → keep (`migrate_legacy_record` below re-keys it
        //     under our session)
        //   - record lacks session_id AND is already terminal → drop
        //     (orphan from before session-id stamping; cannot be attributed)
        match rec.get("session_id").and_then(Value::as_str) {
            Some(sid) if sid == session_id => {}
            Some(_) => continue,
            None => {
                let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
                if status != "allow" && status != "deny" {
                    continue;
                }
            }
        }
        let rec = if let Some(migrated) = migrate_legacy_record(&rec) {
            let call_id = migrated
                .get("function_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !call_id.is_empty() {
                let _ = bus
                    .set(
                        state_scope,
                        &pending_key(session_id, call_id),
                        migrated.clone(),
                    )
                    .await;
            }
            migrated
        } else {
            rec
        };
        let rec = if let Some(flipped) = maybe_flip_timed_out(&rec, now_ms) {
            let call_id = flipped
                .get("function_call_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let _ = bus
                .set(
                    state_scope,
                    &pending_key(session_id, call_id),
                    flipped.clone(),
                )
                .await;
            flipped
        } else {
            rec
        };
        let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
        if !is_terminal_status(status) {
            continue;
        }
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        entries.push(rec);
    }
    entries.sort_by_key(|e| {
        e.get("resolved_at")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });
    let total = entries.len();
    let omitted = total.saturating_sub(limit);
    entries.truncate(limit);
    json!({ "entries": entries, "omitted": omitted })
}

/// Stamp `delivered_in_turn_id` on terminal-status records named in
/// `call_ids` for the given session. Idempotent: records already stamped
/// (non-null `delivered_in_turn_id`) are not overwritten. Unknown call ids
/// are silently skipped.
pub async fn handle_ack_delivered(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
    let call_ids: Vec<String> = payload
        .get("call_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if session_id.is_empty() || turn_id.is_empty() || call_ids.is_empty() {
        return json!({ "ok": true, "stamped": 0 });
    }
    let mut stamped = 0_u64;
    for cid in call_ids {
        let key = pending_key(session_id, &cid);
        let Some(rec) = bus.get(state_scope, &key).await else {
            continue;
        };
        if rec
            .get("delivered_in_turn_id")
            .is_some_and(|v| !v.is_null())
        {
            continue;
        }
        let mut next = rec;
        next.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String(turn_id.to_string()),
        );
        if bus.set(state_scope, &key, next).await.is_ok() {
            stamped += 1;
        }
    }
    json!({ "ok": true, "stamped": stamped })
}

/// Sweep all still-pending approvals for a session to timed_out
/// (reason: session_deleted). Called when a session is being deleted.
pub async fn handle_sweep_session(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id", "swept": 0 });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(state_scope, &prefix).await;
    let mut swept = 0_u64;
    for rec in all {
        if rec.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let call_id = rec
            .get("function_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if call_id.is_empty() {
            continue;
        }
        let flipped = transition_record(
            &rec,
            "timed_out",
            None,
            None,
            Some("session_deleted".into()),
        );
        if bus
            .set(state_scope, &pending_key(session_id, call_id), flipped)
            .await
            .is_ok()
        {
            swept += 1;
        }
    }
    json!({ "ok": true, "swept": swept })
}

fn uuid_like() -> String {
    // Lightweight unique-ish id without pulling uuid in: ns timestamp + counter.
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{n:x}")
}

async fn write_event(iii: &III, session_id: &str, event: &Value) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
                "item_id": format!("approval-{}", uuid_like()),
                "data": event,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

async fn write_hook_reply(iii: &III, stream_name: &str, event_id: &str, reply: &Value) {
    if stream_name.is_empty() || event_id.is_empty() {
        return;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": stream_name,
                "group_id": event_id,
                "item_id": uuid_like(),
                "data": reply,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

/// Production [`StateBus`] backed by a real iii-sdk [`III`] connection.
pub struct IiiStateBus(pub III);

#[async_trait::async_trait]
impl StateBus for IiiStateBus {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": scope, "key": key, "value": value }),
                action: None,
                timeout_ms: None,
            })
            .await
            .map(|_| ())
    }
    async fn get(&self, scope: &str, key: &str) -> Option<Value> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: None,
            })
            .await
            .ok()
            .filter(|v| !v.is_null())
    }
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
        let resp = self
            .0
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": scope, "prefix": prefix }),
                action: None,
                timeout_ms: None,
            })
            .await
            .unwrap_or_else(|_| json!({ "items": [] }));
        // Engine may return either {"items": [...]} or a plain Array.
        if let Some(arr) = resp.as_array() {
            return arr.clone();
        }
        resp.get("items")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.get("value").cloned().unwrap_or(entry))
            .collect()
    }
}

pub fn register(iii: &III, cfg: &WorkerConfig) -> anyhow::Result<Refs> {
    let rules: Arc<Vec<InterceptorRule>> = Arc::new(cfg.interceptors.clone());
    for rule in rules.iter() {
        if let Some(cid) = rule.classifier.as_deref() {
            if cid == FN_LOOKUP_RECORD
                || cid == FN_RESOLVE
                || cid == FN_LIST_PENDING
                || cid == FN_LIST_UNDELIVERED
                || cid == FN_ACK_DELIVERED
                || cid == FN_SWEEP_SESSION
            {
                tracing::warn!(
                    "approval-gate: interceptor for {:?} uses classifier {:?} which aliases an approval endpoint; fix config",
                    rule.function_id,
                    cid
                );
            }
        }
    }

    let bus: Arc<dyn StateBus> = Arc::new(IiiStateBus(iii.clone()));
    let timeout_ms = cfg.default_timeout_ms;
    let topic = cfg.topic.clone();
    let state_scope = cfg.approval_state_scope.clone();

    let bus_for_resolve = bus.clone();
    let scope_resolve = state_scope.clone();
    let exec_for_resolve: Arc<dyn FunctionExecutor> = Arc::new(IiiFunctionExecutor {
        iii: iii.clone(),
        rules: rules.clone(),
    });
    let iii_for_resolve = iii.clone();
    let resolve = iii.register_function((
        RegisterFunctionMessage::with_id(FN_RESOLVE.into()).with_description(
            "Resolve a pending approval. On allow, invokes the underlying function; \
                     on deny, records the denial. The result is stitched into the agent's \
                     next turn as a system message."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_resolve.clone();
            let exec = exec_for_resolve.clone();
            let scope_resolve = scope_resolve.clone();
            let iii = iii_for_resolve.clone();
            async move {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let resp = handle_resolve(
                    bus.as_ref(),
                    exec.as_ref(),
                    &scope_resolve,
                    payload.clone(),
                    now_ms,
                )
                .await;

                if resp.get("ok").and_then(Value::as_bool) == Some(true) {
                    let session_id = payload
                        .get("session_id")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let call_id = payload
                        .get("function_call_id")
                        .or_else(|| payload.get("tool_call_id"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !session_id.is_empty() && !call_id.is_empty() {
                        let key = pending_key(session_id, call_id);
                        if let Some(final_rec) = bus.get(&scope_resolve, &key).await {
                            let mut evt = json!({
                                "type": "approval_resolved",
                                "function_call_id": call_id,
                                "tool_call_id": call_id,
                            });
                            if let Some(status) = final_rec.get("status").and_then(Value::as_str) {
                                evt["decision"] = match status {
                                    "executed" | "approved" => json!("allow"),
                                    _ => json!("deny"),
                                };
                                evt["status"] = json!(status);
                            }
                            if let Some(r) = final_rec.get("result") {
                                evt["result"] = json!(r);
                            }
                            if let Some(e) = final_rec.get("error") {
                                evt["error"] = json!(e);
                            }
                            if let Some(reason) = final_rec.get("decision_reason") {
                                evt["decision_reason"] = json!(reason);
                            }
                            write_event(&iii, session_id, &evt).await;
                        }
                    }
                }
                Ok::<_, IIIError>(resp)
            }
        },
    ));

    let bus_for_list = bus.clone();
    let scope_list = state_scope.clone();
    let list_pending = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LIST_PENDING.into())
            .with_description("Return pending approvals for a session.".into()),
        move |payload: Value| {
            let bus = bus_for_list.clone();
            let scope_list = scope_list.clone();
            async move {
                Ok::<_, IIIError>(handle_list_pending(bus.as_ref(), &scope_list, payload).await)
            }
        },
    ));

    let bus_for_list_undelivered = bus.clone();
    let scope_list_undelivered = state_scope.clone();
    let list_undelivered = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LIST_UNDELIVERED.into()).with_description(
            "Return resolved approval records for a session that haven't yet been stitched \
                 into an LLM turn. Lazy-flips expired pendings to timed_out."
                .into(),
        ),
        move |payload: Value| {
            let bus = bus_for_list_undelivered.clone();
            let scope = scope_list_undelivered.clone();
            async move {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                Ok::<_, IIIError>(
                    handle_list_undelivered(bus.as_ref(), &scope, payload, now_ms).await,
                )
            }
        },
    ));

    let bus_for_ack = bus.clone();
    let scope_ack = state_scope.clone();
    let ack_delivered =
        iii.register_function((
            RegisterFunctionMessage::with_id(FN_ACK_DELIVERED.into()).with_description(
                "Stamp delivered_in_turn_id on resolved approvals so they aren't replayed \
                 in subsequent turns. Idempotent."
                    .into(),
            ),
            move |payload: Value| {
                let bus = bus_for_ack.clone();
                let scope = scope_ack.clone();
                async move {
                    Ok::<_, IIIError>(handle_ack_delivered(bus.as_ref(), &scope, payload).await)
                }
            },
        ));

    let bus_for_sweep = bus.clone();
    let scope_sweep = state_scope.clone();
    let sweep_session =
        iii.register_function((
            RegisterFunctionMessage::with_id(FN_SWEEP_SESSION.into()).with_description(
                "Sweep all pending approvals for a session to timed_out. \
                 Called when a session is deleted."
                    .into(),
            ),
            move |payload: Value| {
                let bus = bus_for_sweep.clone();
                let scope = scope_sweep.clone();
                async move {
                    Ok::<_, IIIError>(handle_sweep_session(bus.as_ref(), &scope, payload).await)
                }
            },
        ));

    let bus_for_lookup = bus.clone();
    let scope_lookup = state_scope.clone();
    let lookup_record =
        iii.register_function((
            RegisterFunctionMessage::with_id(FN_LOOKUP_RECORD.into()).with_description(
                "Return the approval state-store record for a session/function_call_id pair; \
                 null when absent. Used by shell bypass validation."
                    .into(),
            ),
            move |payload: Value| {
                let bus = bus_for_lookup.clone();
                let scope = scope_lookup.clone();
                async move {
                    Ok::<_, IIIError>(handle_lookup_record(bus.as_ref(), &scope, payload).await)
                }
            },
        ));

    let iii_for_sub = iii.clone();
    let bus_for_sub = bus.clone();
    let subscriber_scope = state_scope.clone();
    let rules_for_sub = rules.clone();
    let subscriber_fn = iii.register_function((
        RegisterFunctionMessage::with_id("policy::approval_gate".into())
            .with_description("Pause function calls listed in approval_required.".into()),
        move |envelope: Value| {
            let iii = iii_for_sub.clone();
            let bus = bus_for_sub.clone();
            let sc = subscriber_scope.clone();
            let intercept_rules = rules_for_sub.clone();
            async move {
                let Some(call) = extract_call(&envelope) else {
                    return Ok::<_, IIIError>(json!({ "block": false }));
                };
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                let reply = if call.requires_approval() {
                    match rule_for(intercept_rules.as_slice(), &call.function_id) {
                        Some(rule) if rule.classifier.as_ref().is_some_and(|s| !s.is_empty()) => {
                            let classifier_fn = rule.classifier.as_ref().unwrap();
                            match iii
                                .trigger(TriggerRequest {
                                    function_id: classifier_fn.clone(),
                                    payload: call.args.clone(),
                                    action: None,
                                    timeout_ms: Some(rule.classifier_timeout_ms),
                                })
                                .await
                            {
                                Ok(v) => match interpret_classifier_reply(&v) {
                                    Ok(ClassifierDecision::Auto) => json!({ "block": false }),
                                    Ok(ClassifierDecision::Deny { reason }) => json!({
                                        "block": true,
                                        "reason": format!("approval-classifier: {reason}"),
                                        "status": "denied",
                                        "call_id": call.function_call_id,
                                        "function_id": call.function_id,
                                    }),
                                    Ok(ClassifierDecision::Ask) | Err(()) => {
                                        handle_intercept(
                                            bus.as_ref(),
                                            &sc,
                                            &call,
                                            now_ms,
                                            timeout_ms,
                                            true,
                                        )
                                        .await
                                    }
                                },
                                Err(_) => {
                                    handle_intercept(
                                        bus.as_ref(),
                                        &sc,
                                        &call,
                                        now_ms,
                                        timeout_ms,
                                        true,
                                    )
                                    .await
                                }
                            }
                        }
                        _ => {
                            handle_intercept(bus.as_ref(), &sc, &call, now_ms, timeout_ms, false)
                                .await
                        }
                    }
                } else {
                    json!({ "block": false })
                };

                if reply.get("status").and_then(Value::as_str) == Some("pending") {
                    write_event(
                        &iii,
                        &call.session_id,
                        &json!({
                            "type": "approval_requested",
                            "function_call_id": call.function_call_id,
                            "tool_call_id": call.function_call_id,
                            "function_id": call.function_id,
                            "tool_name": call.function_id,
                            "args": call.args,
                            "expires_at": now_ms.saturating_add(timeout_ms),
                        }),
                    )
                    .await;
                }
                write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                Ok(reply)
            }
        },
    ));

    let subscriber_trigger = iii
        .register_trigger(RegisterTriggerInput {
            trigger_type: "durable:subscriber".into(),
            function_id: "policy::approval_gate".into(),
            config: json!({ "topic": topic }),
            metadata: None,
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(Refs {
        resolve,
        list_pending,
        list_undelivered,
        ack_delivered,
        sweep_session,
        lookup_record,
        subscriber_fn,
        subscriber_trigger,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maybe_flip_timed_out_returns_some_when_pending_and_expired() {
        let rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let flipped = maybe_flip_timed_out(&rec, 70_000).expect("should flip");
        assert_eq!(flipped["status"], "timed_out");
        assert_eq!(flipped["decision_reason"], "timeout");
    }

    #[test]
    fn maybe_flip_timed_out_returns_none_when_pending_and_not_expired() {
        let rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        assert!(maybe_flip_timed_out(&rec, 60_000).is_none());
        assert!(maybe_flip_timed_out(&rec, 1_500).is_none());
    }

    #[test]
    fn maybe_flip_timed_out_returns_none_when_not_pending() {
        let rec = json!({
            "function_call_id": "tc-1",
            "status": "executed",
            "expires_at": 1_000_u64,
        });
        assert!(maybe_flip_timed_out(&rec, 999_999_999).is_none());
    }

    #[test]
    fn transition_record_stamps_resolved_at_for_terminal_status() {
        let base = build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record_with_now(
            &base,
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            12_345,
        );
        assert_eq!(rec["resolved_at"].as_u64(), Some(12_345));
    }

    #[test]
    fn transition_record_preserves_existing_resolved_at_on_relift() {
        let base = build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let first = transition_record_with_now(
            &base,
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            12_345,
        );
        let second = transition_record_with_now(
            &first,
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            99_999,
        );
        assert_eq!(second["resolved_at"].as_u64(), Some(12_345));
    }

    #[test]
    fn transition_record_does_not_stamp_resolved_at_for_intermediate_status() {
        let base = build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec =
            transition_record_with_now(&base, "approved", None, None, None, 12_345);
        assert!(rec.get("resolved_at").is_none());
    }

    #[tokio::test]
    async fn handle_list_undelivered_caps_at_default_limit_and_reports_omitted() {
        let bus = InMemoryStateBus::new();
        for i in 0..75 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 50);
        assert_eq!(resp["omitted"].as_u64(), Some(25));
    }

    #[tokio::test]
    async fn handle_list_undelivered_honors_explicit_limit() {
        let bus = InMemoryStateBus::new();
        for i in 0..10 {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                1_000 + i as u64,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_list_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "limit": 3}),
            100_000,
        )
        .await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 3);
        assert_eq!(resp["omitted"].as_u64(), Some(7));
    }

    #[tokio::test]
    async fn handle_list_undelivered_returns_oldest_first_by_resolved_at() {
        let bus = InMemoryStateBus::new();
        for (i, ts) in [(0_u32, 5_000_u64), (1, 1_000), (2, 3_000)] {
            let cid = format!("c{i}");
            let mut rec = transition_record_with_now(
                &build_pending_record(&cid, "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
                ts,
            );
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), Value::String("s1".into()));
            bus.set(STATE_SCOPE, &pending_key("s1", &cid), rec)
                .await
                .unwrap();
        }
        let resp = handle_list_undelivered(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "limit": 10}),
            100_000,
        )
        .await;
        let entries = resp["entries"].as_array().unwrap();
        let ids: Vec<&str> = entries
            .iter()
            .map(|e| e["function_call_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["c1", "c2", "c0"]);
    }

    #[tokio::test]
    async fn handle_list_undelivered_omitted_is_zero_when_under_limit() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record_with_now(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
            1_500,
        );
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 1);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn handle_list_undelivered_returns_terminal_records_with_no_delivered_stamp() {
        let bus = InMemoryStateBus::new();
        let mut r1 = transition_record(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        r1.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), r1)
            .await
            .unwrap();
        let mut r2 = transition_record(
            &build_pending_record("c2", "shell::fs::write", &json!({}), 1_000, 60_000),
            "denied",
            None,
            None,
            Some("nope".into()),
        );
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c2"), r2)
            .await
            .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(resp["omitted"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn handle_list_undelivered_excludes_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 1_500).await;
        assert_eq!(resp["entries"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn handle_list_undelivered_empty_session_returns_empty() {
        let bus = InMemoryStateBus::new();
        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 1_500).await;
        assert_eq!(resp["entries"], json!([]));
    }

    #[tokio::test]
    async fn handle_list_undelivered_excludes_records_stamped_with_delivered_turn_id() {
        let bus = InMemoryStateBus::new();
        let mut rec = transition_record(
            &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        {
            let obj = rec.as_object_mut().unwrap();
            obj.insert(
                "delivered_in_turn_id".into(),
                Value::String("turn-prev".into()),
            );
            obj.insert("session_id".into(), Value::String("s1".into()));
        }
        bus.set(STATE_SCOPE, &pending_key("s1", "c1"), rec)
            .await
            .unwrap();

        let mut r2 = transition_record(
            &build_pending_record("c2", "shell::fs::write", &json!({}), 1_000, 60_000),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), Value::String("s1".into()));
        bus.set(STATE_SCOPE, &pending_key("s1", "c2"), r2)
            .await
            .unwrap();

        let resp =
            handle_list_undelivered(&bus, STATE_SCOPE, json!({"session_id": "s1"}), 100_000).await;
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["function_call_id"], "c2");
    }

    #[tokio::test]
    async fn handle_list_undelivered_returns_empty_when_session_id_missing() {
        let bus = InMemoryStateBus::new();
        let resp = handle_list_undelivered(&bus, STATE_SCOPE, json!({}), 1_500).await;
        assert_eq!(resp["entries"], json!([]));
    }

    #[tokio::test]
    async fn handle_ack_delivered_stamps_records_with_turn_id() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "call_ids": ["c1"],
                "turn_id": "turn-1",
            }),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"], json!(1));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["delivered_in_turn_id"], "turn-1");
    }

    #[tokio::test]
    async fn handle_ack_delivered_is_idempotent_keeps_first_turn_id() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let _ = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["c1"], "turn_id": "turn-first",
            }),
        )
        .await;
        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["c1"], "turn_id": "turn-second",
            }),
        )
        .await;
        assert_eq!(resp["stamped"], json!(0), "second ack must not re-stamp");

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["delivered_in_turn_id"], "turn-first");
    }

    #[tokio::test]
    async fn handle_ack_delivered_skips_unknown_call_ids_silently() {
        let bus = InMemoryStateBus::new();
        let resp = handle_ack_delivered(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1", "call_ids": ["ghost"], "turn_id": "turn-1",
            }),
        )
        .await;
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["stamped"], json!(0));
    }

    #[tokio::test]
    async fn handle_resolve_on_expired_pending_flips_to_timed_out_and_ignores_decision() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({"session_id":"s1","function_call_id":"tc-1","decision":"allow"}),
            70_000,
        )
        .await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], "timed_out");

        assert!(exec.calls.lock().unwrap().is_empty());

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
    }

    #[test]
    fn migrate_legacy_record_maps_allow_to_executed_without_result() {
        let legacy = json!({
            "function_call_id": "c1",
            "function_id": "shell::fs::write",
            "args": {},
            "status": "allow",
            "expires_at": 1_000_u64,
        });
        let migrated = migrate_legacy_record(&legacy).expect("migrates");
        assert_eq!(migrated["status"], "executed");
        assert!(
            migrated["result"].is_null()
                || migrated.get("result").is_none()
                || migrated["result"] == json!(null)
        );
        assert_eq!(migrated["legacy_migrated"], json!(true));
    }

    #[test]
    fn migrate_legacy_record_maps_deny_to_denied_with_original_reason() {
        let legacy = json!({
            "function_call_id": "c1",
            "status": "deny",
            "reason": "manual",
            "expires_at": 1_000_u64,
        });
        let migrated = migrate_legacy_record(&legacy).expect("migrates");
        assert_eq!(migrated["status"], "denied");
        assert_eq!(migrated["decision_reason"], "manual");
        assert_eq!(migrated["legacy_migrated"], json!(true));
    }

    #[test]
    fn migrate_legacy_record_returns_none_for_new_status_strings() {
        for new_status in [
            "pending",
            "executed",
            "failed",
            "denied",
            "timed_out",
            "approved",
        ] {
            let rec = json!({"status": new_status});
            assert!(
                migrate_legacy_record(&rec).is_none(),
                "should not migrate already-new status '{}'",
                new_status
            );
        }
    }

    #[test]
    fn fn_constants_match_spec_strings() {
        assert_eq!(FN_RESOLVE, "approval::resolve");
        assert_eq!(FN_LIST_PENDING, "approval::list_pending");
        assert_eq!(FN_LIST_UNDELIVERED, "approval::list_undelivered");
        assert_eq!(FN_ACK_DELIVERED, "approval::ack_delivered");
        assert_eq!(FN_LOOKUP_RECORD, "approval::lookup_record");
    }

    #[test]
    fn interpret_classifier_reply_reads_decision_tags() {
        assert!(matches!(
            interpret_classifier_reply(&json!({"decision": "auto"})),
            Ok(ClassifierDecision::Auto)
        ));
        match interpret_classifier_reply(&json!({"decision":"deny","reason":"nope"})) {
            Ok(ClassifierDecision::Deny { reason }) => assert_eq!(reason, "nope"),
            o => panic!("expected deny {:?}", o),
        }
        assert!(matches!(
            interpret_classifier_reply(&json!({"decision":"ask","summary":"x"})),
            Ok(ClassifierDecision::Ask)
        ));
        assert!(interpret_classifier_reply(&json!({})).is_err());
    }

    #[test]
    fn merge_from_approval_inserts_marker_when_inject_true() {
        let m = merge_from_approval_marker_if_needed(
            true,
            json!({"command": "git"}),
            "call-1",
            "sess-1",
        );
        let inner = m.get("__from_approval").unwrap();
        assert_eq!(inner["call_id"], "call-1");
        assert_eq!(inner["session_id"], "sess-1");
        assert_eq!(m["command"], "git");
    }

    #[test]
    fn merge_from_approval_noop_when_inject_false() {
        let j = json!({"a": 1});
        let out = merge_from_approval_marker_if_needed(false, j.clone(), "c", "s");
        assert_eq!(out, j);
    }

    #[test]
    fn rule_for_returns_matching_rule() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: Some("shell::classify_argv".into()),
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
            },
            InterceptorRule {
                function_id: "other::fn".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
            },
        ];
        let r = rule_for(&rules, "shell::exec").expect("match");
        assert_eq!(r.classifier.as_deref(), Some("shell::classify_argv"));
        assert!(r.inject_approval_marker);
    }

    #[test]
    fn rule_for_returns_none_when_absent() {
        let rules = vec![InterceptorRule {
            function_id: "x::y".into(),
            classifier: None,
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
        }];
        assert!(rule_for(&rules, "missing::id").is_none());
    }

    #[test]
    fn is_terminal_status_returns_true_for_terminal_states() {
        assert!(is_terminal_status("executed"));
        assert!(is_terminal_status("failed"));
        assert!(is_terminal_status("denied"));
        assert!(is_terminal_status("timed_out"));
    }

    #[test]
    fn is_terminal_status_returns_false_for_in_progress_states() {
        assert!(!is_terminal_status("pending"));
        assert!(!is_terminal_status("approved"));
        assert!(!is_terminal_status("anything_else"));
        assert!(!is_terminal_status(""));
    }

    #[test]
    fn pending_key_includes_session_and_tool_call_id() {
        assert_eq!(pending_key("s1", "tc-1"), "s1/tc-1");
    }

    #[test]
    fn extract_call_reads_session_id_and_function_call_from_envelope() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "function_call": { "id": "tc-1", "function_id": "write", "arguments": {"path": "/tmp/x"} },
                "approval_required": ["write"],
                "session_id": "s1",
            }
        });
        let call = extract_call(&envelope).expect("decoded");
        assert_eq!(call.session_id, "s1");
        assert_eq!(call.function_call_id, "tc-1");
        assert_eq!(call.function_id, "write");
        assert_eq!(call.event_id, "evt-1");
        assert_eq!(call.reply_stream, "rs-1");
        assert!(call.approval_required.iter().any(|s| s == "write"));
    }

    #[test]
    fn extract_call_accepts_legacy_tool_call_envelope_with_name() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "tool_call": { "id": "tc-1", "name": "write", "arguments": {} },
                "approval_required": ["write"],
                "session_id": "s1",
            }
        });
        let call = extract_call(&envelope).expect("decoded");
        assert_eq!(call.function_call_id, "tc-1");
        assert_eq!(call.function_id, "write");
    }

    #[test]
    fn requires_approval_only_for_listed_functions() {
        let call = IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "ls".into(),
            args: json!({}),
            approval_required: vec!["write".into()],
            event_id: "e".into(),
            reply_stream: "r".into(),
        };
        assert!(!call.requires_approval());

        let call2 = IncomingCall {
            function_id: "write".into(),
            ..call
        };
        assert!(call2.requires_approval());
    }

    #[test]
    fn build_pending_record_sets_status_and_expiry() {
        let now = 1_000_000;
        let rec = build_pending_record("tc-1", "write", &json!({"x": 1}), now, 60_000);
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 1_060_000);
    }

    #[test]
    fn block_reply_for_decision_allow_does_not_block() {
        let reply = block_reply_for(&Decision::Allow);
        assert_eq!(reply["block"], false);
    }

    #[test]
    fn block_reply_for_deny_includes_reason() {
        let reply = block_reply_for(&Decision::Deny {
            reason: "timeout".into(),
        });
        assert_eq!(reply["block"], true);
        assert_eq!(reply["reason"], "approval-gate: timeout");
    }

    #[test]
    fn extract_call_returns_none_when_function_call_absent() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": { "session_id": "s1", "approval_required": ["write"] }
        });
        assert!(extract_call(&envelope).is_none());
    }

    #[test]
    fn extract_call_returns_none_when_session_id_absent() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "tool_call": { "id": "tc-1", "name": "write", "arguments": {} }
            }
        });
        assert!(extract_call(&envelope).is_none());
    }

    #[test]
    fn block_reply_for_allow_omits_reason() {
        let reply = block_reply_for(&Decision::Allow);
        assert_eq!(reply["block"], false);
        assert!(
            reply.get("reason").is_none(),
            "Allow must not include reason: {reply}"
        );
    }

    use std::sync::Mutex;

    fn sample_call() -> IncomingCall {
        IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "shell::fs::write".into(),
            args: json!({"path": "/tmp/a"}),
            approval_required: vec!["shell::fs::write".into()],
            event_id: "evt-1".into(),
            reply_stream: "rs-1".into(),
        }
    }

    #[tokio::test]
    async fn handle_intercept_returns_pending_envelope_when_call_is_gated() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("pending"));
        assert_eq!(reply["call_id"], json!("tc-1"));
        assert_eq!(reply["function_id"], json!("shell::fs::write"));
        assert_eq!(reply["reason"], json!("approval-gate: pending_approval"));
    }

    #[tokio::test]
    async fn handle_intercept_writes_pending_record_to_state() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let key = pending_key(&call.session_id, &call.function_call_id);
        let rec = bus
            .get(STATE_SCOPE, &key)
            .await
            .expect("pending record written");
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 61_000);
    }

    #[tokio::test]
    async fn handle_intercept_passes_through_when_call_is_not_gated() {
        let bus = InMemoryStateBus::new();
        let mut call = sample_call();
        call.approval_required = vec!["other".into()];
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(false));
        let key = pending_key(&call.session_id, &call.function_call_id);
        assert!(
            bus.get(STATE_SCOPE, &key).await.is_none(),
            "no record written"
        );
    }

    #[tokio::test]
    async fn handle_intercept_force_pending_writes_when_not_on_required_list() {
        let bus = InMemoryStateBus::new();
        let mut call = sample_call();
        call.approval_required = vec!["other".into()];
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, true).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("pending"));
        let key = pending_key(&call.session_id, &call.function_call_id);
        assert!(bus.get(STATE_SCOPE, &key).await.is_some());
    }

    #[tokio::test]
    async fn handle_lookup_record_returns_null_when_missing() {
        let bus = InMemoryStateBus::new();
        let v = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "function_call_id": "c1"}),
        )
        .await;
        assert!(v.is_null());
    }

    #[tokio::test]
    async fn handle_lookup_record_returns_record_when_present() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let v = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s1", "function_call_id": "tc-1"}),
        )
        .await;
        assert_eq!(v["status"], json!("pending"));
        assert_eq!(v["function_id"], json!("shell::fs::write"));
    }

    #[derive(Default)]
    struct FakeExecutor {
        calls: Mutex<Vec<(String, Value, String, String)>>,
        response: Mutex<Option<Result<Value, String>>>,
    }

    #[async_trait::async_trait]
    impl FunctionExecutor for FakeExecutor {
        async fn invoke(
            &self,
            function_id: &str,
            args: Value,
            function_call_id: &str,
            session_id: &str,
        ) -> Result<Value, String> {
            self.calls.lock().unwrap().push((
                function_id.to_string(),
                args,
                function_call_id.to_string(),
                session_id.to_string(),
            ));
            self.response
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| Ok(json!({"ok": true})))
        }
    }

    #[tokio::test]
    async fn handle_resolve_allow_invokes_function_and_records_executed() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record(
                "tc-1",
                "shell::fs::write",
                &json!({"path":"/a"}),
                1_000,
                60_000,
            ),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));

        let calls = exec.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "shell::fs::write");
        assert_eq!(calls[0].1, json!({"path":"/a"}));
        assert_eq!(calls[0].2, "tc-1");
        assert_eq!(calls[0].3, "s1");

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "executed");
        assert_eq!(rec["result"], json!({"ok": true}));
    }

    #[tokio::test]
    async fn handle_resolve_deny_does_not_invoke_function() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "reason": "not authorized",
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));

        assert!(exec.calls.lock().unwrap().is_empty());

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "denied");
        assert_eq!(rec["decision_reason"], "not authorized");
    }

    #[tokio::test]
    async fn handle_resolve_allow_records_failed_when_function_errors() {
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        *exec.response.lock().unwrap() = Some(Err("EACCES".into()));
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({"session_id":"s1","function_call_id":"tc-1","decision":"allow"}),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], json!(true));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "failed");
        assert_eq!(rec["error"], "EACCES");
    }

    #[tokio::test]
    async fn fake_executor_records_calls() {
        let exec = FakeExecutor::default();
        let out = exec
            .invoke("shell::fs::write", json!({"x": 1}), "cid", "sid")
            .await
            .unwrap();
        assert_eq!(out, json!({"ok": true}));
        let calls = exec.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "shell::fs::write");
        assert_eq!(calls[0].2, "cid");
        assert_eq!(calls[0].3, "sid");
    }

    struct InMemoryStateBus {
        store: Mutex<std::collections::HashMap<String, Value>>,
    }

    impl InMemoryStateBus {
        fn new() -> Self {
            Self {
                store: Mutex::new(std::collections::HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl StateBus for InMemoryStateBus {
        async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
            self.store
                .lock()
                .unwrap()
                .insert(format!("{scope}/{key}"), value);
            Ok(())
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.store
                .lock()
                .unwrap()
                .get(&format!("{scope}/{key}"))
                .cloned()
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            let map = self.store.lock().unwrap();
            map.iter()
                .filter(|(k, _)| k.starts_with(&format!("{scope}/{prefix}")))
                .map(|(_, v)| v.clone())
                .collect()
        }
    }

    #[tokio::test]
    async fn resolve_flips_status_when_pending() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({
                "function_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;

        assert_eq!(out["ok"], true);
        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "executed");
    }

    #[tokio::test]
    async fn resolve_accepts_legacy_tool_call_id_field() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({
                "tool_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;

        assert_eq!(out["ok"], true);
    }

    #[tokio::test]
    async fn resolve_rejects_already_resolved_entry() {
        let bus = InMemoryStateBus::new();
        let mut rec = build_pending_record("tc-1", "write", &json!({}), 0, 60_000);
        rec["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({"function_call_id": "tc-1", "session_id": "s1", "decision": "deny"}),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "already_resolved");
    }

    #[tokio::test]
    async fn list_pending_returns_only_pending_for_session() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();
        let mut resolved = build_pending_record("tc-2", "write", &json!({}), 0, 60_000);
        resolved["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), resolved)
            .await
            .unwrap();
        bus.set(
            STATE_SCOPE,
            &pending_key("other", "tc-3"),
            build_pending_record("tc-3", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let out = handle_list_pending(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["function_call_id"], "tc-1");
    }

    #[tokio::test]
    async fn resolve_deny_records_reason() {
        let bus = InMemoryStateBus::new();
        let _ = bus
            .set(
                STATE_SCOPE,
                &pending_key("s1", "tc-1"),
                build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
            )
            .await;

        let exec = FakeExecutor::default();
        let out = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "reason": "user clicked cancel",
            }),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], true);

        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "denied");
        assert_eq!(stored["decision_reason"], "user clicked cancel");
    }

    #[test]
    fn transition_record_to_executed_attaches_result() {
        let base = build_pending_record(
            "tc-1",
            "shell::fs::write",
            &json!({"path":"/a"}),
            1_000,
            60_000,
        );
        let rec = transition_record(&base, "executed", Some(json!({"ok": true})), None, None);
        assert_eq!(rec["status"], "executed");
        assert_eq!(rec["result"], json!({"ok": true}));
        assert!(rec.get("error").is_none() || rec["error"].is_null());
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["function_id"], "shell::fs::write");
    }

    #[test]
    fn transition_record_to_failed_attaches_error() {
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(&base, "failed", None, Some("EACCES".into()), None);
        assert_eq!(rec["status"], "failed");
        assert_eq!(rec["error"], "EACCES");
        assert!(rec.get("result").is_none() || rec["result"].is_null());
    }

    #[test]
    fn transition_record_to_denied_attaches_decision_reason() {
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(&base, "denied", None, None, Some("not authorized".into()));
        assert_eq!(rec["status"], "denied");
        assert_eq!(rec["decision_reason"], "not authorized");
    }

    #[test]
    fn transition_record_to_timed_out_uses_timeout_reason() {
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(&base, "timed_out", None, None, Some("timeout".into()));
        assert_eq!(rec["status"], "timed_out");
        assert_eq!(rec["decision_reason"], "timeout");
    }

    #[test]
    fn transition_record_preserves_delivered_in_turn_id_when_set() {
        let mut base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        base.as_object_mut().unwrap().insert(
            "delivered_in_turn_id".into(),
            Value::String("turn-X".into()),
        );
        let rec = transition_record(&base, "executed", Some(json!({"ok": true})), None, None);
        assert_eq!(rec["delivered_in_turn_id"], "turn-X");
    }

    #[tokio::test]
    async fn handle_sweep_session_flips_pending_records_to_timed_out_with_reason_session_deleted() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
        )
        .await
        .unwrap();

        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({"session_id": "s1"})).await;
        assert_eq!(resp["swept"], json!(1));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "timed_out");
        assert_eq!(rec["decision_reason"], "session_deleted");
    }

    #[tokio::test]
    async fn handle_sweep_session_skips_non_pending_records() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "c1"),
            transition_record(
                &build_pending_record("c1", "shell::fs::write", &json!({}), 1_000, 60_000),
                "executed",
                Some(json!({"ok": true})),
                None,
                None,
            ),
        )
        .await
        .unwrap();

        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({"session_id": "s1"})).await;
        assert_eq!(resp["swept"], json!(0));

        let rec = bus
            .get(STATE_SCOPE, &pending_key("s1", "c1"))
            .await
            .unwrap();
        assert_eq!(rec["status"], "executed");
    }

    #[tokio::test]
    async fn handle_sweep_session_returns_error_when_session_id_missing() {
        let bus = InMemoryStateBus::new();
        let resp = handle_sweep_session(&bus, STATE_SCOPE, json!({})).await;
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["error"], "missing_session_id");
        assert_eq!(resp["swept"], json!(0));
    }
}
