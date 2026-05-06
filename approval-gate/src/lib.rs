//! Approval gate. Subscribes to `agent::before_tool_call` and blocks calls
//! whose `tool_call.name` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

use std::sync::Arc;

use iii_sdk::{
    FunctionRef, IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, III,
};
use serde_json::{json, Value};

pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const STATE_SCOPE: &str = "approvals";
pub const DEFAULT_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub topic: String,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            topic: "agent::before_tool_call".into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(t) = std::env::var("APPROVAL_GATE_TIMEOUT_MS") {
            match t.parse::<u64>() {
                Ok(n) => cfg.timeout_ms = n,
                Err(err) => log::warn!(
                    "APPROVAL_GATE_TIMEOUT_MS={t:?} is not a valid u64 ({err}); \
                     using default {DEFAULT_TIMEOUT_MS}ms"
                ),
            }
        }
        cfg
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingCall {
    pub session_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
    pub approval_required: Vec<String>,
    pub event_id: String,
    pub reply_stream: String,
}

impl IncomingCall {
    pub fn requires_approval(&self) -> bool {
        self.approval_required.iter().any(|n| n == &self.tool_name)
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
/// `session_id` and `tool_call_id` must not contain `/`. They are caller-controlled
/// IDs minted by turn-orchestrator; today neither format uses the separator.
pub fn pending_key(session_id: &str, tool_call_id: &str) -> String {
    debug_assert!(!session_id.contains('/'), "session_id must not contain '/'");
    debug_assert!(!tool_call_id.contains('/'), "tool_call_id must not contain '/'");
    format!("{session_id}/{tool_call_id}")
}

pub fn extract_call(envelope: &Value) -> Option<IncomingCall> {
    let event_id = envelope.get("event_id").and_then(Value::as_str)?.to_string();
    let reply_stream = envelope
        .get("reply_stream")
        .and_then(Value::as_str)?
        .to_string();
    let inner = envelope.get("payload").unwrap_or(envelope);
    let session_id = inner
        .get("session_id")
        .and_then(Value::as_str)?
        .to_string();
    let tc = inner.get("tool_call")?;
    Some(IncomingCall {
        session_id,
        tool_call_id: tc.get("id").and_then(Value::as_str)?.to_string(),
        tool_name: tc.get("name").and_then(Value::as_str)?.to_string(),
        args: tc.get("arguments").cloned().unwrap_or_else(|| json!({})),
        approval_required: inner
            .get("approval_required")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        event_id,
        reply_stream,
    })
}

pub fn build_pending_record(
    tool_call_id: &str,
    tool_name: &str,
    args: &Value,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    json!({
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
        "args": args,
        "status": "pending",
        "expires_at": now_ms.saturating_add(timeout_ms),
    })
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
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
}

#[async_trait::async_trait]
pub trait StateBus: Send + Sync {
    async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError>;
    async fn get(&self, scope: &str, key: &str) -> Option<Value>;
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value>;
}

pub async fn handle_resolve(bus: &dyn StateBus, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_call_id = payload
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || tool_call_id.is_empty() {
        return json!({ "ok": false, "error": "missing_id" });
    }
    let Some(decision) = payload
        .get("decision")
        .cloned()
        .and_then(|v| serde_json::from_value::<WireDecision>(v).ok())
    else {
        return json!({ "ok": false, "error": "bad_decision" });
    };
    let key = pending_key(session_id, tool_call_id);
    let Some(mut existing) = bus.get(STATE_SCOPE, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };
    if existing.get("status").and_then(Value::as_str) != Some("pending") {
        return json!({ "ok": false, "error": "already_resolved" });
    }
    existing["status"] = serde_json::to_value(decision)
        .expect("WireDecision serializes via Serialize");
    if let Some(reason) = payload.get("reason").cloned() {
        existing["reason"] = reason;
    }
    if let Err(err) = bus.set(STATE_SCOPE, &key, existing).await {
        log::error!("approval-gate: failed to write resolved state: {err}");
        return json!({ "ok": false, "error": "state_write_failed" });
    }
    json!({ "ok": true })
}

pub async fn handle_list_pending(bus: &dyn StateBus, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "pending": [] });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(STATE_SCOPE, &prefix).await;
    let pending: Vec<Value> = all
        .into_iter()
        .filter(|v| v.get("status").and_then(Value::as_str) == Some("pending"))
        .collect();
    json!({ "pending": pending })
}

const POLL_INTERVAL_MS: u64 = 250;

pub async fn await_decision(
    bus: &dyn StateBus,
    session_id: &str,
    tool_call_id: &str,
    expires_at: u64,
) -> Decision {
    let key = pending_key(session_id, tool_call_id);
    loop {
        let Some(rec) = bus.get(STATE_SCOPE, &key).await else {
            return Decision::Deny {
                reason: "state_unavailable".into(),
            };
        };
        match rec.get("status").and_then(Value::as_str) {
            Some("allow") => return Decision::Allow,
            Some("deny") => {
                let reason = rec
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .to_string();
                return Decision::Deny { reason };
            }
            _ => {}
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(expires_at);
        if now >= expires_at {
            return Decision::Deny {
                reason: "timeout".into(),
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
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
        resp.get("items")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.get("value").cloned().unwrap_or(entry))
            .collect()
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

pub fn register(iii: &III, config: Config) -> anyhow::Result<Refs> {
    let bus: Arc<dyn StateBus> = Arc::new(IiiStateBus(iii.clone()));

    let bus_for_resolve = bus.clone();
    let resolve = iii.register_function((
        RegisterFunctionMessage::with_id(FN_RESOLVE.into())
            .with_description("Flip a pending approval entry to allow or deny.".into()),
        move |payload: Value| {
            let bus = bus_for_resolve.clone();
            async move { Ok::<_, IIIError>(handle_resolve(bus.as_ref(), payload).await) }
        },
    ));

    let bus_for_list = bus.clone();
    let list_pending = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LIST_PENDING.into())
            .with_description("Return pending approvals for a session.".into()),
        move |payload: Value| {
            let bus = bus_for_list.clone();
            async move { Ok::<_, IIIError>(handle_list_pending(bus.as_ref(), payload).await) }
        },
    ));

    let timeout_ms = config.timeout_ms;
    let topic = config.topic.clone();
    let iii_for_sub = iii.clone();
    let bus_for_sub = bus.clone();
    let subscriber_fn = iii.register_function((
        RegisterFunctionMessage::with_id("policy::approval_gate".into())
            .with_description("Pause tool calls listed in approval_required.".into()),
        move |envelope: Value| {
            let iii = iii_for_sub.clone();
            let bus = bus_for_sub.clone();
            async move {
                let Some(call) = extract_call(&envelope) else {
                    return Ok::<_, IIIError>(json!({ "block": false }));
                };
                if !call.requires_approval() {
                    let reply = json!({ "block": false });
                    write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                    return Ok(reply);
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let expires_at = now.saturating_add(timeout_ms);
                let record = build_pending_record(
                    &call.tool_call_id,
                    &call.tool_name,
                    &call.args,
                    now,
                    timeout_ms,
                );
                if let Err(err) = bus
                    .set(
                        STATE_SCOPE,
                        &pending_key(&call.session_id, &call.tool_call_id),
                        record,
                    )
                    .await
                {
                    log::error!(
                        "approval-gate: failed to write pending record for {}/{}: {err}",
                        call.session_id, call.tool_call_id
                    );
                    let reply = json!({ "block": false });
                    write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                    return Ok(reply);
                }
                write_event(
                    &iii,
                    &call.session_id,
                    &json!({
                        "type": "approval_requested",
                        "tool_call_id": call.tool_call_id,
                        "tool_name": call.tool_name,
                        "args": call.args,
                        "expires_at": expires_at,
                    }),
                )
                .await;
                let decision =
                    await_decision(bus.as_ref(), &call.session_id, &call.tool_call_id, expires_at)
                        .await;
                let (decision_str, reason_for_event) = match &decision {
                    Decision::Allow => ("allow", None),
                    Decision::Deny { reason } => ("deny", Some(reason.clone())),
                };
                write_event(
                    &iii,
                    &call.session_id,
                    &json!({
                        "type": "approval_resolved",
                        "tool_call_id": call.tool_call_id,
                        "decision": decision_str,
                        "reason": reason_for_event,
                    }),
                )
                .await;
                let reply = block_reply_for(&decision);
                write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                Ok(reply)
            }
        },
    ));

    let subscriber_trigger = iii
        .register_trigger(RegisterTriggerInput {
            trigger_type: "subscribe".into(),
            function_id: "policy::approval_gate".into(),
            config: json!({ "topic": topic }),
            metadata: None,
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(Refs {
        resolve,
        list_pending,
        subscriber_fn,
        subscriber_trigger,
    })
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
    fn extract_call_reads_session_id_and_tool_call_from_envelope() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "tool_call": { "id": "tc-1", "name": "write", "arguments": {"path": "/tmp/x"} },
                "approval_required": ["write"],
                "session_id": "s1",
            }
        });
        let call = extract_call(&envelope).expect("decoded");
        assert_eq!(call.session_id, "s1");
        assert_eq!(call.tool_call_id, "tc-1");
        assert_eq!(call.tool_name, "write");
        assert_eq!(call.event_id, "evt-1");
        assert_eq!(call.reply_stream, "rs-1");
        assert!(call.approval_required.iter().any(|s| s == "write"));
    }

    #[test]
    fn requires_approval_only_for_listed_tools() {
        let call = IncomingCall {
            session_id: "s1".into(),
            tool_call_id: "tc-1".into(),
            tool_name: "ls".into(),
            args: json!({}),
            approval_required: vec!["write".into()],
            event_id: "e".into(),
            reply_stream: "r".into(),
        };
        assert!(!call.requires_approval());

        let call2 = IncomingCall {
            tool_name: "write".into(),
            ..call
        };
        assert!(call2.requires_approval());
    }

    #[test]
    fn build_pending_record_sets_status_and_expiry() {
        let now = 1_000_000;
        let rec = build_pending_record("tc-1", "write", &json!({"x": 1}), now, 60_000);
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["tool_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 1_060_000);
    }

    #[test]
    fn block_reply_for_decision_allow_does_not_block() {
        let reply = block_reply_for(&Decision::Allow);
        assert_eq!(reply["block"], false);
    }

    #[test]
    fn block_reply_for_deny_includes_reason() {
        let reply = block_reply_for(&Decision::Deny { reason: "timeout".into() });
        assert_eq!(reply["block"], true);
        assert_eq!(reply["reason"], "approval-gate: timeout");
    }

    #[test]
    fn extract_call_returns_none_when_tool_call_absent() {
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
        assert!(reply.get("reason").is_none(), "Allow must not include reason: {reply}");
    }

    use std::sync::Mutex;

    struct InMemoryStateBus {
        store: Mutex<std::collections::HashMap<String, Value>>,
    }

    impl InMemoryStateBus {
        fn new() -> Self {
            Self { store: Mutex::new(std::collections::HashMap::new()) }
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
            self.store.lock().unwrap().get(&format!("{scope}/{key}")).cloned()
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

        let out = handle_resolve(
            &bus,
            json!({
                "tool_call_id": "tc-1",
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
        assert_eq!(stored["status"], "allow");
    }

    #[tokio::test]
    async fn resolve_rejects_already_resolved_entry() {
        let bus = InMemoryStateBus::new();
        let mut rec = build_pending_record("tc-1", "write", &json!({}), 0, 60_000);
        rec["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();

        let out = handle_resolve(
            &bus,
            json!({"tool_call_id": "tc-1", "session_id": "s1", "decision": "deny"}),
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

        let out = handle_list_pending(&bus, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["tool_call_id"], "tc-1");
    }

    use std::sync::Arc;
    use std::time::Duration;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[tokio::test]
    async fn await_decision_returns_allow_when_status_flips() {
        let bus = Arc::new(InMemoryStateBus::new());
        let key = pending_key("s1", "tc-1");
        bus.set(
            STATE_SCOPE,
            &key,
            build_pending_record("tc-1", "write", &json!({}), now_ms(), 5_000),
        )
        .await
        .unwrap();

        let bus2 = bus.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut rec = bus2.get(STATE_SCOPE, &key).await.unwrap();
            rec["status"] = json!("allow");
            bus2.set(STATE_SCOPE, &key, rec).await.unwrap();
        });

        let decision = await_decision(&*bus, "s1", "tc-1", now_ms() + 5_000).await;
        writer.await.unwrap();
        assert_eq!(decision, Decision::Allow);
    }

    #[tokio::test]
    async fn await_decision_returns_deny_timeout_when_expired() {
        let bus = InMemoryStateBus::new();
        let key = pending_key("s1", "tc-1");
        bus.set(
            STATE_SCOPE,
            &key,
            build_pending_record("tc-1", "write", &json!({}), 0, 0),
        )
        .await;
        let decision = await_decision(&bus, "s1", "tc-1", now_ms() - 10).await;
        match decision {
            Decision::Deny { reason } => assert_eq!(reason, "timeout"),
            other => panic!("expected Deny(timeout), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_decision_fail_closed_on_missing_record() {
        let bus = InMemoryStateBus::new();
        let decision = await_decision(&bus, "s1", "tc-1", now_ms() + 1_000).await;
        match decision {
            Decision::Deny { reason } => assert_eq!(reason, "state_unavailable"),
            other => panic!("expected Deny(state_unavailable), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_deny_records_reason() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await;

        let out = handle_resolve(
            &bus,
            json!({
                "session_id": "s1",
                "tool_call_id": "tc-1",
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
        assert_eq!(stored["status"], "deny");
        assert_eq!(stored["reason"], "user clicked cancel");
    }
}
