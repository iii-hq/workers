//! Record-lifecycle helpers: build_pending_record, transition_record,
//! maybe_flip_timed_out, collect_timed_out_for_sweep, plus the small
//! is_terminal_status / pending_key utilities.

mod common;

use approval_gate::*;
use common::{empty_policy_rules, sample_call, FailingStateBus, FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::Mutex;



    #[test]
    fn maybe_flip_timed_out_returns_some_when_pending_and_expired() {
        let rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let flipped = maybe_flip_timed_out(&rec, 70_000).expect("should flip");
        assert_eq!(flipped["status"], "timed_out");
        // Timeout carries no Denial — the status alone explains the outcome.
        assert!(flipped.get("denial").is_none());
        assert!(flipped.get("decision_reason").is_none());
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
    fn build_pending_record_sets_status_and_expiry() {
        let now = 1_000_000;
        let rec = build_pending_record("tc-1", "write", &json!({"x": 1}), now, 60_000);
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 1_060_000);
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
    fn transition_record_to_denied_attaches_structured_denial() {
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(
            &base,
            "denied",
            None,
            None,
            Some(Denial::Policy {
                rule_permission: "shell::fs::write".into(),
                rule_pattern: "*".into(),
            }),
        );
        assert_eq!(rec["status"], "denied");
        assert_eq!(rec["denial"]["kind"], "policy");
        assert_eq!(rec["denial"]["detail"]["rule_permission"], "shell::fs::write");
        assert!(
            rec.get("decision_reason").is_none(),
            "legacy decision_reason must not be written: {rec}"
        );
    }


    #[test]
    fn transition_record_to_timed_out_carries_no_denial() {
        // Timeout status is self-describing — no Denial attached.
        let base = build_pending_record("tc-1", "shell::fs::write", &json!({}), 1_000, 60_000);
        let rec = transition_record(&base, "timed_out", None, None, None);
        assert_eq!(rec["status"], "timed_out");
        assert!(rec.get("denial").is_none());
        assert!(rec.get("decision_reason").is_none());
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


    #[test]
    fn collect_timed_out_for_sweep_returns_expired_records_with_session_id() {
        let mut rec = build_pending_record("tc-1", "shell::fs::write", &json!({}), 0, 60_000);
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s-42"));
        let pile = vec![
            rec.clone(),
            build_pending_record("tc-2", "shell::fs::write", &json!({}), 0, 999_999_999),
        ];
        let out = collect_timed_out_for_sweep(&pile, 70_000);
        assert_eq!(out.len(), 1);
        let (key, flipped, session_id, call_id) = &out[0];
        assert_eq!(key, "s-42/tc-1");
        assert_eq!(session_id, "s-42");
        assert_eq!(call_id, "tc-1");
        assert_eq!(flipped["status"], json!("timed_out"));
        // Timeout carries no Denial — status is self-describing.
        assert!(flipped.get("denial").is_none());
        assert!(flipped.get("decision_reason").is_none());
    }


    #[test]
    fn collect_timed_out_for_sweep_skips_records_without_session_id() {
        // Legacy row (pre-session_id-stamping fix). The sweeper can't
        // address the right session stream, so it must skip silently —
        // lazy-flip on read will still pick it up.
        let pile = vec![build_pending_record(
            "tc-legacy",
            "shell::fs::write",
            &json!({}),
            0,
            60_000,
        )];
        let out = collect_timed_out_for_sweep(&pile, 70_000);
        assert!(
            out.is_empty(),
            "legacy record without session_id must not be swept"
        );
    }


    #[test]
    fn collect_timed_out_for_sweep_rejects_record_missing_only_call_id() {
        // mutant L423: `||` → `&&` would let one-empty records sweep.
        let mut rec = build_pending_record("c1", "shell::fs::write", &json!({}), 0, 60_000);
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        rec.as_object_mut()
            .unwrap()
            .insert("function_call_id".into(), json!(""));
        let out = collect_timed_out_for_sweep(&[rec], 70_000);
        assert!(out.is_empty(), "empty function_call_id must skip sweep");
    }


    #[test]
    fn maybe_flip_timed_out_flips_at_exact_expires_at() {
        // mutant L439: `<` → `<=` would not flip at the exact boundary.
        let rec = build_pending_record("c1", "f", &json!({}), 0, 60_000);
        // expires_at = 0 + 60_000 = 60_000. At now=60_000 the gate
        // considers the record expired (strictly past or AT expiry).
        assert!(
            maybe_flip_timed_out(&rec, 60_000).is_some(),
            "must flip at exactly expires_at"
        );
        assert!(
            maybe_flip_timed_out(&rec, 59_999).is_none(),
            "must not flip one ms before expires_at"
        );
    }
