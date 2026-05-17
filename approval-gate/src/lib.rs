//! Approval gate. Subscribes to `agent::before_function_call` and blocks calls
//! whose `function_call.function_id` appears in the run's `approval_required` list,
//! then lets `approval::consume` drain resolved decisions back into the turn.

pub mod config;
pub mod manifest;

pub use config::WorkerConfig;

use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};

pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const FN_CONSUME: &str = "approval::consume";
pub const FN_SWEEP_SESSION: &str = "approval::sweep_session";
/// Default `approval_state_scope` (matches [`WorkerConfig::default`]).
pub const STATE_SCOPE: &str = "approvals";

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
    session_id: &str,
    function_call_id: &str,
    function_id: &str,
    args: &Value,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    json!({
        "session_id": session_id,
        "function_call_id": function_call_id,
        "function_id": function_id,
        "args": args,
        "status": "pending",
        "created_at": now_ms,
        "expires_at": now_ms.saturating_add(timeout_ms),
    })
}

pub fn block_reply_for(decision: &Decision) -> Value {
    match decision {
        Decision::Allow => json!({
            "block": false,
            "subscriber": "approval-gate",
            "approval_gate": true,
        }),
        Decision::Deny { reason } => json!({
            "block": true,
            "reason": format!("approval-gate: {reason}"),
            "subscriber": "approval-gate",
            "approval_gate": true,
        }),
    }
}

pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub consume: FunctionRef,
    pub sweep_session: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
}

#[async_trait::async_trait]
pub trait StateBus: Send + Sync {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError>;
    async fn get(&self, scope: &str, key: &str) -> Option<Value>;
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value>;
}

pub async fn handle_resolve(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
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
    let Some(decision) = payload
        .get("decision")
        .cloned()
        .and_then(|v| serde_json::from_value::<WireDecision>(v).ok())
    else {
        return json!({ "ok": false, "error": "bad_decision" });
    };
    let key = pending_key(session_id, function_call_id);
    let Some(mut existing) = bus.get(state_scope, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };
    if existing.get("status").and_then(Value::as_str) != Some("pending") {
        return json!({ "ok": false, "error": "already_resolved" });
    }
    let now_ms = now_ms();
    if existing
        .get("expires_at")
        .and_then(Value::as_u64)
        .is_some_and(|expires_at| now_ms >= expires_at)
    {
        existing["status"] = json!("resolved");
        existing["decision"] = json!("deny");
        existing["reason"] = json!("timed_out");
        existing["resolved_at"] = json!(now_ms);
        let _ = bus.set(state_scope, &key, existing).await;
        return json!({ "ok": false, "error": "timed_out" });
    }
    existing["status"] = json!("resolved");
    existing["decision"] =
        serde_json::to_value(decision).expect("WireDecision serializes via Serialize");
    existing["resolved_at"] = json!(now_ms);
    if let Some(reason) = payload.get("reason").cloned() {
        existing["reason"] = reason;
    }
    if let Err(err) = bus.set(state_scope, &key, existing).await {
        tracing::error!("approval-gate: failed to write resolved state: {err}");
        return json!({ "ok": false, "error": "state_write_failed" });
    }
    json!({ "ok": true })
}

pub async fn handle_intercept(
    bus: &dyn StateBus,
    state_scope: &str,
    call: &IncomingCall,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    if !call.requires_approval() {
        return block_reply_for(&Decision::Allow);
    }

    let record = build_pending_record(
        &call.session_id,
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
        return json!({
            "block": true,
            "status": "denied",
            "reason": "approval-gate: state_write_failed",
            "subscriber": "approval-gate",
            "approval_gate": true,
            "denial": {
                "kind": "state_error",
                "detail": {
                    "phase": "pending_write",
                    "error": err.to_string(),
                }
            }
        });
    }

    json!({
        "block": true,
        "status": "pending",
        "reason": "approval required",
        "function_call_id": call.function_call_id,
        "tool_call_id": call.function_call_id,
        "function_id": call.function_id,
        "subscriber": "approval-gate",
        "approval_gate": true,
    })
}

pub async fn handle_consume(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id", "entries": [] });
    }

    let prefix = format!("{session_id}/");
    let rows = bus.list_prefix(state_scope, &prefix).await;
    let mut entries = Vec::new();
    for mut row in rows {
        if row.get("session_id").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        if row.get("status").and_then(Value::as_str) != Some("resolved") {
            continue;
        }
        let Some(function_call_id) = row.get("function_call_id").and_then(Value::as_str) else {
            continue;
        };
        let key = pending_key(session_id, function_call_id);
        entries.push(json!({
            "function_call_id": function_call_id,
            "tool_call_id": function_call_id,
            "function_id": row.get("function_id").cloned().unwrap_or(Value::Null),
            "args": row.get("args").cloned().unwrap_or_else(|| json!({})),
            "decision": row.get("decision").cloned().unwrap_or_else(|| json!("deny")),
            "reason": row.get("reason").cloned().unwrap_or(Value::Null),
        }));
        row["status"] = json!("consumed");
        row["consumed_at"] = json!(now_ms());
        let _ = bus.set(state_scope, &key, row).await;
    }
    json!({ "ok": true, "entries": entries })
}

pub async fn handle_sweep_session(bus: &dyn StateBus, state_scope: &str, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "ok": false, "error": "missing_session_id", "swept": 0 });
    }

    let prefix = format!("{session_id}/");
    let rows = bus.list_prefix(state_scope, &prefix).await;
    let now_ms = now_ms();
    let mut swept = 0u64;
    for mut row in rows {
        if row.get("session_id").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        if row.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let Some(function_call_id) = row
            .get("function_call_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        row["status"] = json!("resolved");
        row["decision"] = json!("deny");
        row["reason"] = json!("timed_out");
        row["resolved_at"] = json!(now_ms);
        if bus
            .set(
                state_scope,
                &pending_key(session_id, &function_call_id),
                row,
            )
            .await
            .is_ok()
        {
            swept += 1;
        }
    }
    json!({ "ok": true, "swept": swept })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
        .filter(|v| v.get("session_id").and_then(Value::as_str) == Some(session_id))
        .filter(|v| v.get("status").and_then(Value::as_str) == Some("pending"))
        .collect();
    json!({ "pending": pending })
}

fn state_list_values(resp: Value) -> Vec<Value> {
    let entries = match resp {
        Value::Array(entries) => entries,
        Value::Object(mut obj) => obj
            .remove("items")
            .and_then(|items| match items {
                Value::Array(entries) => Some(entries),
                _ => None,
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    entries
        .into_iter()
        .map(|entry| entry.get("value").cloned().unwrap_or(entry))
        .collect()
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
        state_list_values(resp)
    }
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

pub fn register(iii: &III, cfg: &WorkerConfig) -> anyhow::Result<Refs> {
    let bus: Arc<dyn StateBus> = Arc::new(IiiStateBus(iii.clone()));
    let timeout_ms = cfg.default_timeout_ms;
    let topic = cfg.topic.clone();
    let state_scope = cfg.approval_state_scope.clone();

    let bus_for_resolve = bus.clone();
    let scope_resolve = state_scope.clone();
    let iii_for_resolve = iii.clone();
    let resolve = iii.register_function((
        RegisterFunctionMessage::with_id(FN_RESOLVE.into())
            .with_description("Flip a pending approval entry to allow or deny.".into()),
        move |payload: Value| {
            let bus = bus_for_resolve.clone();
            let scope_resolve = scope_resolve.clone();
            let iii = iii_for_resolve.clone();
            async move {
                let session_id = payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let function_call_id = payload
                    .get("function_call_id")
                    .or_else(|| payload.get("tool_call_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let out = handle_resolve(bus.as_ref(), &scope_resolve, payload).await;
                if out.get("ok").and_then(Value::as_bool) == Some(true)
                    && !session_id.is_empty()
                    && !function_call_id.is_empty()
                {
                    if let Some(record) = bus
                        .get(&scope_resolve, &pending_key(&session_id, &function_call_id))
                        .await
                    {
                        write_event(
                            &iii,
                            &session_id,
                            &approval_resolved_event(&function_call_id, &record),
                        )
                        .await;
                    }
                    if let Err(err) = resume_session(&iii, &session_id).await {
                        write_event(
                            &iii,
                            &session_id,
                            &json!({
                                "type": "approval_wake_failed",
                                "error": err,
                            }),
                        )
                        .await;
                    }
                }
                Ok::<_, IIIError>(out)
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

    let bus_for_consume = bus.clone();
    let scope_consume = state_scope.clone();
    let consume =
        iii.register_function((
            RegisterFunctionMessage::with_id(FN_CONSUME.into())
                .with_description("Return resolved approval decisions for a session once.".into()),
            move |payload: Value| {
                let bus = bus_for_consume.clone();
                let scope_consume = scope_consume.clone();
                async move {
                    Ok::<_, IIIError>(handle_consume(bus.as_ref(), &scope_consume, payload).await)
                }
            },
        ));

    let bus_for_sweep = bus.clone();
    let scope_sweep = state_scope.clone();
    let sweep_session = iii.register_function((
        RegisterFunctionMessage::with_id(FN_SWEEP_SESSION.into())
            .with_description("Resolve a session's pending approvals as denied.".into()),
        move |payload: Value| {
            let bus = bus_for_sweep.clone();
            let scope_sweep = scope_sweep.clone();
            async move {
                Ok::<_, IIIError>(handle_sweep_session(bus.as_ref(), &scope_sweep, payload).await)
            }
        },
    ));

    let iii_for_sub = iii.clone();
    let bus_for_sub = bus.clone();
    let subscriber_scope = state_scope.clone();
    let subscriber_fn = iii.register_function((
        RegisterFunctionMessage::with_id("policy::approval_gate".into())
            .with_description("Pause function calls listed in approval_required.".into()),
        move |envelope: Value| {
            let iii = iii_for_sub.clone();
            let bus = bus_for_sub.clone();
            let sc = subscriber_scope.clone();
            async move {
                let Some(call) = extract_call(&envelope) else {
                    let reply = block_reply_for(&Decision::Allow);
                    return Ok(reply);
                };
                let now = now_ms();
                let reply = handle_intercept(bus.as_ref(), &sc, &call, now, timeout_ms).await;
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
                            "expires_at": now.saturating_add(timeout_ms),
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
        consume,
        sweep_session,
        subscriber_fn,
        subscriber_trigger,
    })
}

fn approval_resolved_event(function_call_id: &str, record: &Value) -> Value {
    json!({
        "type": "approval_resolved",
        "function_call_id": function_call_id,
        "tool_call_id": function_call_id,
        "decision": record.get("decision").cloned().unwrap_or_else(|| json!("deny")),
        "reason": record.get("reason").cloned().unwrap_or(Value::Null),
    })
}

async fn resume_session(iii: &III, session_id: &str) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let response = iii
            .trigger(TriggerRequest {
                function_id: "run::resume".into(),
                payload: json!({ "session_id": session_id }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
            .map_err(|err| err.to_string())?;

        if response.get("resumed").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            return Err("run::resume did not reopen approval turn before timeout".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let rec = build_pending_record("s1", "tc-1", "write", &json!({"x": 1}), now, 60_000);
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
            build_pending_record("s1", "tc-1", "write", &json!({}), now_ms(), 60_000),
        )
        .await
        .unwrap();

        let out = handle_resolve(
            &bus,
            STATE_SCOPE,
            json!({
                "function_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
        )
        .await;

        assert_eq!(out["ok"], true);
        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "resolved");
        assert_eq!(stored["decision"], "allow");
    }

    #[tokio::test]
    async fn intercept_required_call_writes_pending_and_returns_marked_pending_block() {
        let bus = InMemoryStateBus::new();
        let call = IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "shell::exec".into(),
            args: json!({"command": "date"}),
            approval_required: vec!["shell::exec".into()],
            event_id: "evt".into(),
            reply_stream: "replies".into(),
        };

        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000).await;

        assert_eq!(reply["block"], true);
        assert_eq!(reply["status"], "pending");
        assert_eq!(reply["subscriber"], "approval-gate");
        assert_eq!(reply["approval_gate"], true);
        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "pending");
        assert_eq!(stored["session_id"], "s1");
        assert_eq!(stored["function_id"], "shell::exec");
    }

    #[tokio::test]
    async fn intercept_non_required_call_returns_marked_allow_reply() {
        let bus = InMemoryStateBus::new();
        let call = IncomingCall {
            session_id: "s1".into(),
            function_call_id: "tc-1".into(),
            function_id: "shell::fs::ls".into(),
            args: json!({}),
            approval_required: vec!["shell::exec".into()],
            event_id: "evt".into(),
            reply_stream: "replies".into(),
        };

        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000).await;

        assert_eq!(reply["block"], false);
        assert_eq!(reply["subscriber"], "approval-gate");
        assert_eq!(reply["approval_gate"], true);
        assert!(bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .is_none());
    }

    #[tokio::test]
    async fn consume_returns_resolved_entries_once_and_marks_consumed() {
        let bus = InMemoryStateBus::new();
        let mut rec = build_pending_record(
            "s1",
            "tc-1",
            "shell::exec",
            &json!({"command": "date"}),
            0,
            60_000,
        );
        rec["status"] = json!("resolved");
        rec["decision"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();

        let first = handle_consume(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        assert_eq!(first["ok"], true);
        assert_eq!(first["entries"].as_array().unwrap().len(), 1);
        assert_eq!(first["entries"][0]["decision"], "allow");

        let second = handle_consume(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        assert_eq!(second["entries"].as_array().unwrap().len(), 0);

        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "consumed");
    }

    #[tokio::test]
    async fn sweep_session_resolves_pending_as_deny_and_prevents_later_allow() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("s1", "tc-1", "shell::exec", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let sweep = handle_sweep_session(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        assert_eq!(sweep["ok"], true);
        assert_eq!(sweep["swept"], 1);

        let allow = handle_resolve(
            &bus,
            STATE_SCOPE,
            json!({ "session_id": "s1", "function_call_id": "tc-1", "decision": "allow" }),
        )
        .await;
        assert_eq!(allow["ok"], false);
        assert_eq!(allow["error"], "already_resolved");
    }

    #[tokio::test]
    async fn resolve_accepts_legacy_tool_call_id_field() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("s1", "tc-1", "write", &json!({}), now_ms(), 60_000),
        )
        .await
        .unwrap();

        let out = handle_resolve(
            &bus,
            STATE_SCOPE,
            json!({
                "tool_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
        )
        .await;

        assert_eq!(out["ok"], true);
    }

    #[tokio::test]
    async fn resolve_rejects_already_resolved_entry() {
        let bus = InMemoryStateBus::new();
        let mut rec = build_pending_record("s1", "tc-1", "write", &json!({}), 0, 60_000);
        rec["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();

        let out = handle_resolve(
            &bus,
            STATE_SCOPE,
            json!({"function_call_id": "tc-1", "session_id": "s1", "decision": "deny"}),
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
            build_pending_record("s1", "tc-1", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();
        let mut resolved = build_pending_record("s1", "tc-2", "write", &json!({}), 0, 60_000);
        resolved["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), resolved)
            .await
            .unwrap();
        bus.set(
            STATE_SCOPE,
            &pending_key("other", "tc-3"),
            build_pending_record("other", "tc-3", "write", &json!({}), 0, 60_000),
        )
        .await
        .unwrap();

        let out = handle_list_pending(&bus, STATE_SCOPE, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["function_call_id"], "tc-1");
    }

    #[test]
    fn state_list_values_accepts_raw_state_array() {
        let out = state_list_values(json!([
            {
                "session_id": "s1",
                "function_call_id": "tc-1",
                "status": "pending"
            }
        ]));

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["function_call_id"], "tc-1");
    }

    #[test]
    fn state_list_values_accepts_items_envelope_and_unwraps_values() {
        let out = state_list_values(json!({
            "items": [
                {
                    "key": "s1/tc-1",
                    "value": {
                        "session_id": "s1",
                        "function_call_id": "tc-1",
                        "status": "pending"
                    }
                }
            ]
        }));

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["function_call_id"], "tc-1");
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[tokio::test]
    async fn resolve_deny_records_reason() {
        let bus = InMemoryStateBus::new();
        let _ = bus
            .set(
                STATE_SCOPE,
                &pending_key("s1", "tc-1"),
                build_pending_record("s1", "tc-1", "write", &json!({}), now_ms(), 60_000),
            )
            .await;

        let out = handle_resolve(
            &bus,
            STATE_SCOPE,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "reason": "user clicked cancel",
            }),
        )
        .await;
        assert_eq!(out["ok"], true);

        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "resolved");
        assert_eq!(stored["decision"], "deny");
        assert_eq!(stored["reason"], "user clicked cancel");
    }
}
