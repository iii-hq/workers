//! Approval gate. Subscribes to `agent::before_tool_call` and blocks calls
//! whose `tool_call.name` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

use iii_sdk::{FunctionRef, III};
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
    async fn set(&self, scope: &str, key: &str, value: Value);
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
    let decision = payload
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || tool_call_id.is_empty() {
        return json!({ "ok": false, "error": "missing_id" });
    }
    if decision != "allow" && decision != "deny" {
        return json!({ "ok": false, "error": "bad_decision" });
    }
    let key = pending_key(session_id, tool_call_id);
    let Some(mut existing) = bus.get(STATE_SCOPE, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };
    if existing.get("status").and_then(Value::as_str) != Some("pending") {
        return json!({ "ok": false, "error": "already_resolved" });
    }
    existing["status"] = json!(decision);
    if let Some(reason) = payload.get("reason").cloned() {
        existing["reason"] = reason;
    }
    bus.set(STATE_SCOPE, &key, existing).await;
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

pub fn register(_iii: &III, _config: Config) -> anyhow::Result<Refs> {
    anyhow::bail!("not yet implemented")
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
        async fn set(&self, scope: &str, key: &str, value: Value) {
            self.store
                .lock()
                .unwrap()
                .insert(format!("{scope}/{key}"), value);
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
        .await;

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
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec).await;

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
        .await;
        let mut resolved = build_pending_record("tc-2", "write", &json!({}), 0, 60_000);
        resolved["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), resolved).await;
        bus.set(
            STATE_SCOPE,
            &pending_key("other", "tc-3"),
            build_pending_record("tc-3", "write", &json!({}), 0, 60_000),
        )
        .await;

        let out = handle_list_pending(&bus, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["tool_call_id"], "tc-1");
    }
}
