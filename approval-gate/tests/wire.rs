//! Wire-shape helpers: extract_call envelope parsing, block_reply_for
//! hook reply, IncomingCall::requires_approval semantics.

mod common;

use approval_gate::*;
use common::{empty_policy_rules, sample_call, FailingStateBus, FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::Mutex;



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
    fn block_reply_for_policy_deny_carries_classifier_detail() {
        let reply = block_reply_for(&Decision::Deny(Denial::Policy {
            classifier_reason: "command matches denylist".into(),
            classifier_fn: "shell::classify_argv".into(),
        }));
        assert_eq!(reply["block"], true);
        assert_eq!(reply["denial"]["kind"], "policy");
        assert_eq!(
            reply["denial"]["detail"]["classifier_reason"],
            "command matches denylist"
        );
        assert_eq!(
            reply["denial"]["detail"]["classifier_fn"],
            "shell::classify_argv"
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
