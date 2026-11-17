//! `workflow::stamp-reply` — the pre_call hook that stamps the caller's session
//! and folder into `workflow::start`.
//!
//! Observes `harness::hook::on-pre-call` scoped to `workflow::start` (see
//! `configuration::observe_pre_call_hook`). It stamps the TRUE caller session,
//! taken from the harness-supplied hook envelope, as `caller_session_id`,
//! OVERWRITING anything the agent supplied: node sessions nest under it, and
//! `workflow::start` answers it with the `next` observe call. It also replaces
//! any supplied `caller_fs_root` with the caller turn's folder
//! (`metadata.fs_scope.root`), so node sessions run where the caller runs. If
//! there is nothing to stamp or remove, the hook answers a bare `continue` with
//! no mutations (the harness parses that identically to `null`).
//!
//! The observer is bound `on_error: fail_closed`: a hook error or timeout
//! refuses `workflow::start`, since an unstamped call would carry the agent's
//! own `caller_fs_root`. The hook runs only on calls made from an agent turn:
//! the harness refuses `workflow::start` as a binding target and `fp` refuses it
//! as a pipe step, since both dispatch without it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STAMP_REPLY_ID: &str = "workflow::stamp-reply";

/// Hook outcome returned to the harness: always `continue`, with rewritten
/// `workflow::start` arguments when it stamps the caller's session or folder, or
/// removes a supplied `caller_fs_root`.
/// The harness's `parse_output` treats a missing/`"continue"` decision and
/// absent mutations as "continue unchanged", so the no-op case is the same
/// wire the old `null` reply produced — but the response schema stays typed
/// (the registry publish gate refuses AnyValue response schemas).
#[derive(Debug, Serialize, JsonSchema)]
pub struct StampReplyResponse {
    /// Always `"continue"` — this hook never denies or holds a call.
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutations: Option<StampReplyMutations>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StampReplyMutations {
    /// The call's arguments, rewritten with the caller's stamped session and folder
    /// (any supplied `caller_fs_root` removed).
    pub arguments: Value,
}

fn cont(mutations: Option<StampReplyMutations>) -> StampReplyResponse {
    StampReplyResponse {
        decision: "continue".to_string(),
        mutations,
    }
}

/// The slice of the `pre_call` hook envelope we need (lenient: ignores the
/// other envelope fields the harness sends).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct StampReplyEvent {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub call: Option<StampCall>,
    /// Caller turn's `options.metadata`; its `fs_scope.root` is the caller's
    /// folder, stamped as `caller_fs_root`.
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StampCall {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

/// Pure decision: `continue` with rewritten arguments when there was a caller
/// identity or folder to stamp, or a supplied folder to remove, else
/// `continue` with no mutations (unchanged call).
pub fn decide(event: StampReplyEvent) -> StampReplyResponse {
    let mut args = event.call.map(|c| c.arguments).unwrap_or(Value::Null);
    let Some(obj) = args.as_object_mut() else {
        return cont(None);
    };
    // The folder comes from the caller's turn, never the agent: drop a supplied
    // one even when there is no session to stamp.
    let mut changed = obj.remove("caller_fs_root").is_some();
    let root = event
        .metadata
        .as_ref()
        .and_then(|m| m.pointer("/fs_scope/root"))
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty());
    if let Some(root) = root {
        obj.insert("caller_fs_root".into(), json!(root));
        changed = true;
    }
    let Some(session_id) = event.session_id.filter(|s| !s.is_empty()) else {
        // No caller identity to stamp: a supplied one is the agent's, never trusted.
        changed |= obj.remove("caller_session_id").is_some();
        return cont(changed.then_some(StampReplyMutations { arguments: args }));
    };
    // Always record the orchestrator session: node sessions nest under it in the
    // console tree, and the run's outcome is observed from it. Identity comes from
    // the hook envelope, never the agent, so this overwrites any supplied value.
    obj.insert("caller_session_id".into(), json!(session_id));
    cont(Some(StampReplyMutations { arguments: args }))
}

/// Pre_call hook handler: stamp the caller identity into the call's arguments
/// (see `decide`) and ALWAYS continue. The hook never parks or blocks the harness
/// turn — `workflow::start` is fire-and-forget; the outcome comes back through an
/// observe of `workflow::on-run-completed` or `notify`.
pub async fn handle(event: StampReplyEvent) -> Result<StampReplyResponse, iii_sdk::errors::Error> {
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

    /// The decision as the harness sees it on the wire.
    fn wire(response: StampReplyResponse) -> Value {
        serde_json::to_value(response).expect("response serializes")
    }

    #[test]
    fn leaves_reply_to_alone_and_stamps_the_session_and_folder() {
        // `reply_to` was removed: `workflow::start` refuses it, so the hook passes
        // it through untouched instead of stamping an identity into it.
        let reply_to = json!({ "session_id": "victim", "template": "done:" });
        let out = wire(decide(
            serde_json::from_value(json!({
                "session_id": "real_caller",
                "metadata": { "fs_scope": { "root": "/home/u/proj" } },
                "call": { "arguments": {
                    "reply_to": reply_to,
                    "caller_session_id": "victim"
                } },
            }))
            .expect("envelope"),
        ));
        let args = &out["mutations"]["arguments"];
        assert_eq!(args["reply_to"], reply_to);
        assert_eq!(args["caller_session_id"], "real_caller"); // overwritten
        assert_eq!(args["caller_fs_root"], "/home/u/proj");
    }

    #[test]
    fn stamps_caller_session_on_a_plain_start() {
        // Capture the caller session so the run's node sessions can nest under
        // the orchestrator chat, and nothing else.
        let out = wire(decide(event(
            "sess_1",
            "m1",
            json!({ "definition": { "version": 1 } }),
        )));
        assert_eq!(out["decision"], "continue");
        let args = &out["mutations"]["arguments"];
        assert_eq!(args["caller_session_id"], "sess_1");
        assert!(args.get("reply_to").is_none());
    }

    #[test]
    fn drops_a_supplied_caller_session_when_the_envelope_has_none() {
        let mut e = event(
            "",
            "",
            json!({"result": {"n": 1}, "caller_session_id": "wf_r_x_other@r0"}),
        );
        e.session_id = None;
        let args = wire(decide(e))["mutations"]["arguments"].clone();
        assert!(args.get("caller_session_id").is_none(), "{args}");
        assert_eq!(args["result"], json!({"n": 1}));
    }

    #[test]
    fn no_op_without_caller_session() {
        let ev: StampReplyEvent =
            serde_json::from_value(json!({ "call": { "arguments": { "reply_to": {} } } })).unwrap();
        // A bare continue with no mutations key — the harness parses this
        // identically to the old `null` reply (continue unchanged).
        assert_eq!(wire(decide(ev)), json!({ "decision": "continue" }));
    }

    #[test]
    fn stamps_caller_fs_root_from_envelope_metadata() {
        // Forge attempt: the agent names another folder; the caller turn's wins.
        let out = wire(decide(
            serde_json::from_value(json!({
                "session_id": "s_orch",
                "metadata": { "fs_scope": { "root": "/home/u/proj" } },
                "call": { "arguments": { "definition": {}, "caller_fs_root": "/etc" } },
            }))
            .expect("envelope"),
        ));
        let args = &out["mutations"]["arguments"];
        assert_eq!(args["caller_fs_root"], "/home/u/proj");
        assert_eq!(args["caller_session_id"], "s_orch");
    }

    #[test]
    fn removes_supplied_caller_fs_root_when_envelope_has_none() {
        let args = json!({ "definition": {}, "caller_fs_root": "/etc" });
        let out = wire(decide(event("s_orch", "m1", args.clone())));
        assert!(out["mutations"]["arguments"]
            .get("caller_fs_root")
            .is_none());
        // The removal runs without a session id too.
        let out = wire(decide(
            serde_json::from_value(json!({ "call": { "arguments": args } })).expect("envelope"),
        ));
        assert_eq!(
            out["mutations"]["arguments"],
            json!({ "definition": {} }),
            "{out}"
        );
    }

    /// Mirrors the registry publish gate (`collect_worker_interface.py`): the
    /// derived response schema must carry a schema-defining keyword, not the
    /// AnyValue schema.
    #[test]
    fn response_schema_passes_the_publish_typed_gate() {
        let schema = schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<StampReplyResponse>();
        let value = serde_json::to_value(schema).expect("schema serializes");
        let obj = value.as_object().expect("schema is an object");
        assert!(
            ["type", "properties", "$ref"]
                .iter()
                .any(|k| obj.contains_key(*k)),
            "StampReplyResponse schema is untyped: {value}"
        );
    }
}
