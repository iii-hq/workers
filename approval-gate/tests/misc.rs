//! Miscellaneous: function-id constants, marker-target validation,
//! and the FakeExecutor recording-of-calls smoke test.

mod common;

use approval_gate::*;
use common::{empty_policy_rules, sample_call, FailingStateBus, FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::Mutex;



    #[test]
    fn fn_constants_match_spec_strings() {
        assert_eq!(FN_RESOLVE, "approval::resolve");
        assert_eq!(FN_LIST_PENDING, "approval::list_pending");
        assert_eq!(FN_LIST_UNDELIVERED, "approval::list_undelivered");
        assert_eq!(FN_ACK_DELIVERED, "approval::ack_delivered");
        assert_eq!(FN_LOOKUP_RECORD, "approval::lookup_record");
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


    #[test]
    fn unverified_marker_targets_lists_unasserted_rules() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: false,
            },
            InterceptorRule {
                function_id: "shell::exec_bg".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: true,
            },
            InterceptorRule {
                function_id: "no_marker::fn".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
                marker_target_verified: false,
            },
        ];
        assert_eq!(unverified_marker_targets(&rules), vec!["shell::exec"]);
    }


    #[test]
    fn unverified_marker_targets_empty_when_all_verified_or_marker_off() {
        let rules = vec![
            InterceptorRule {
                function_id: "shell::exec".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: true,
                marker_target_verified: true,
            },
            InterceptorRule {
                function_id: "other".into(),
                classifier: None,
                classifier_timeout_ms: 2000,
                inject_approval_marker: false,
                marker_target_verified: false,
            },
        ];
        assert!(unverified_marker_targets(&rules).is_empty());
    }
