//! Wire-shape helpers: extract_call envelope parsing and block_reply_for
//! hook replies. Legacy `approval_required` is parsed for tolerance but
//! does not drive policy.

use approval_gate::rules::{Action, RuleMatch, RuleSource};
use approval_gate::*;
use serde_json::json;

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
fn block_reply_for_decision_allow_does_not_block() {
    let reply = block_reply_for(&Decision::Allow);
    assert_eq!(reply["block"], false);
}

#[test]
fn block_reply_for_deny_emits_structured_denial() {
    let reply = block_reply_for(&Decision::Deny(Denial::UserRejected));
    assert_eq!(reply["block"], true);
    assert_eq!(reply["denial"]["kind"], "user_rejected");
    assert!(reply.as_object().unwrap().get("reason").is_none());
}

#[test]
fn block_reply_for_approval_rule_deny_carries_rule_detail() {
    let reply = block_reply_for(&Decision::Deny(Denial::ApprovalRuleDenied {
        rule: RuleMatch {
            source: RuleSource::Global,
            index: 0,
            permission: "shell::exec".into(),
            pattern: "rm -rf*".into(),
            action: Action::Deny,
            reason: Some("destructive command".into()),
        },
    }));
    assert_eq!(reply["block"], true);
    assert_eq!(reply["denial"]["kind"], "approval_rule_denied");
    assert_eq!(
        reply["denial"]["detail"]["rule"]["permission"],
        "shell::exec"
    );
    assert_eq!(reply["denial"]["detail"]["rule"]["pattern"], "rm -rf*");
    assert_eq!(
        reply["denial"]["detail"]["rule"]["reason"],
        "destructive command"
    );
}

#[test]
fn block_reply_for_user_corrected_carries_feedback() {
    let reply = block_reply_for(&Decision::Deny(Denial::UserCorrected {
        feedback: "use git diff instead".into(),
    }));
    assert_eq!(reply["denial"]["kind"], "user_corrected");
    assert_eq!(
        reply["denial"]["detail"]["feedback"],
        "use git diff instead"
    );
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
fn parse_call_error_preserves_reply_routing_when_call_fields_are_bad() {
    let envelope = json!({
        "event_id": "evt-1",
        "reply_stream": "rs-1",
        "payload": { "session_id": "s1" }
    });
    let err = parse_call(&envelope).unwrap_err();
    assert_eq!(err.phase, "extract_call");
    assert_eq!(err.event_id.as_deref(), Some("evt-1"));
    assert_eq!(err.reply_stream.as_deref(), Some("rs-1"));
    assert!(err.error.contains("function_call"));
}

#[test]
fn parse_call_error_without_reply_routing_can_only_hard_block_locally() {
    let envelope = json!({
        "payload": {
            "session_id": "s1",
            "function_call": { "id": "tc-1", "function_id": "write", "arguments": {} }
        }
    });
    let err = parse_call(&envelope).unwrap_err();
    assert_eq!(err.event_id, None);
    assert_eq!(err.reply_stream, None);
    assert!(err.error.contains("event_id"));
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
fn block_reply_for_allow_omits_denial_and_reason() {
    let reply = block_reply_for(&Decision::Allow);
    assert_eq!(reply["block"], false);
    assert!(
        reply.get("reason").is_none(),
        "Allow must not include reason: {reply}"
    );
    assert!(
        reply.get("denial").is_none(),
        "Allow must not include denial: {reply}"
    );
}
