//! `workflow::stamp-reply` — the pre_trigger hook that makes `reply_to`
//! auto-capture the caller's session.
//!
//! Bound to `harness::hook::pre-trigger` scoped to `workflow::start` (see
//! `configuration::bind_pre_trigger_hook`). When an agent calls `workflow::start`
//! with a `reply_to` object, this hook stamps the TRUE caller identity
//! (`session_id` / `model` / `provider`, taken from the harness-supplied hook
//! envelope) into `reply_to`, OVERWRITING anything the agent supplied — so an
//! agent cannot direct a run's result into another session. If the call has no
//! `reply_to`, the hook is a no-op (returns `null` = continue unchanged).
//!
//! The harness routes deserialization failures / hook errors through the
//! binding's `on_error: fail_open`, so a hiccup here never blocks
//! `workflow::start` — it just skips the auto-stamp.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

pub const STAMP_REPLY_ID: &str = "workflow::stamp-reply";

/// The slice of the `pre_trigger` hook envelope we need (lenient: ignores the
/// other envelope fields the harness sends).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StampReplyEvent {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub call: Option<StampCall>,
    /// Caller turn's dispatch policy from the hook envelope, captured into a stamped
    /// `reply_to` so reply delivery can wake the caller with its own reach.
    #[serde(default)]
    pub functions: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StampCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

/// Pure decision: return the `continue` outcome with rewritten arguments when
/// the call opted into `reply_to`, else `Value::Null` (continue unchanged).
pub fn decide(event: StampReplyEvent) -> Value {
    let Some(session_id) = event.session_id.filter(|s| !s.is_empty()) else {
        return Value::Null; // no caller identity to stamp
    };
    let mut args = event.call.map(|c| c.arguments).unwrap_or(Value::Null);
    let Some(obj) = args.as_object_mut() else {
        return Value::Null;
    };
    // Always record the orchestrator session so node sessions can nest under it
    // in the console tree — independent of reply_to/await. Identity comes from
    // the hook envelope, never the agent, so this overwrites any supplied value.
    obj.insert("caller_session_id".into(), json!(session_id));
    // If the caller opted into reply_to, stamp the true identity into it too.
    if obj.contains_key("reply_to") {
        // Carry over a caller-supplied template; overwrite identity from the envelope.
        let template = obj.get("reply_to").and_then(|r| r.get("template")).cloned();
        let mut reply = serde_json::Map::new();
        reply.insert("session_id".into(), json!(session_id));
        if let Some(m) = event.model.filter(|s| !s.is_empty()) {
            reply.insert("model".into(), json!(m));
        }
        if let Some(p) = event.provider.filter(|s| !s.is_empty()) {
            reply.insert("provider".into(), json!(p));
        }
        // Caller's dispatch policy from the envelope (identity-stamped, never from
        // the agent), so reply delivery can wake the caller with its own reach.
        if let Some(f) = event.functions {
            reply.insert("functions".into(), f);
        }
        if let Some(t) = template {
            reply.insert("template".into(), t);
        }
        obj.insert("reply_to".into(), Value::Object(reply));
    }
    json!({ "decision": "continue", "mutations": { "arguments": args } })
}

/// Pre_trigger hook handler: stamp the caller identity into the call's arguments
/// (see `decide`) and ALWAYS continue. The hook never parks or blocks the harness
/// turn — `workflow::start` is fire-and-forget; the outcome comes back via
/// `reply_to` / `notify`.
pub async fn handle(event: StampReplyEvent) -> Result<Value, iii_sdk::errors::Error> {
    Ok(decide(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(session: &str, model: &str, args: Value) -> StampReplyEvent {
        serde_json::from_value(json!({
            "session_id": session,
            "model": model,
            "call": { "arguments": args },
        }))
        .expect("envelope")
    }

    #[test]
    fn stamps_session_and_model_when_reply_to_present() {
        let out = decide(event(
            "sess_1",
            "m1",
            json!({
                "definition": { "version": 1 },
                "reply_to": {}
            }),
        ));
        let args = &out["mutations"]["arguments"];
        let rt = &args["reply_to"];
        assert_eq!(out["decision"], "continue");
        assert_eq!(rt["session_id"], "sess_1");
        assert_eq!(rt["model"], "m1");
        // Caller session is also captured for tree-nesting.
        assert_eq!(args["caller_session_id"], "sess_1");
    }

    #[test]
    fn captures_caller_function_policy_into_reply_to() {
        // The envelope carries the caller turn's dispatch policy; decide stamps it
        // into reply_to so delivery can wake the caller with its own reach.
        let out = decide(
            serde_json::from_value(json!({
                "session_id": "sess_1",
                "model": "m1",
                "functions": { "deny": ["workflow::*"] },
                "call": { "arguments": { "definition": { "version": 1 }, "reply_to": {} } },
            }))
            .expect("envelope"),
        );
        let rt = &out["mutations"]["arguments"]["reply_to"];
        assert!(rt["functions"].get("allow").is_none());
        assert_eq!(rt["functions"]["deny"][0], "workflow::*");
    }

    #[test]
    fn overwrites_agent_supplied_session_id() {
        // Spoof attempt: agent points reply_to at someone else's session.
        let out = decide(event(
            "real_caller",
            "m1",
            json!({
                "reply_to": { "session_id": "victim", "template": "done:" }
            }),
        ));
        let rt = &out["mutations"]["arguments"]["reply_to"];
        assert_eq!(rt["session_id"], "real_caller"); // overwritten
        assert_eq!(rt["template"], "done:"); // template preserved
    }

    #[test]
    fn stamps_caller_session_when_reply_to_absent() {
        // Fire-and-forget start (no reply_to): still capture the caller session
        // so the run's node sessions can nest under the orchestrator chat.
        let out = decide(event(
            "sess_1",
            "m1",
            json!({ "definition": { "version": 1 } }),
        ));
        assert_eq!(out["decision"], "continue");
        let args = &out["mutations"]["arguments"];
        assert_eq!(args["caller_session_id"], "sess_1");
        assert!(args.get("reply_to").is_none()); // no reply_to manufactured
    }

    #[test]
    fn no_op_without_caller_session() {
        let ev: StampReplyEvent =
            serde_json::from_value(json!({ "call": { "arguments": { "reply_to": {} } } })).unwrap();
        assert!(decide(ev).is_null());
    }
}
