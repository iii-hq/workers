//! Approval-resolve flow: handle_resolve, the cascade-on-`always` sweep,
//! and handle_lookup_record. Uses an InMemoryStateBus + FakeExecutor.

mod common;

use approval_gate::*;
use common::{empty_policy_rules, sample_call, FailingStateBus, FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::Mutex;



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
            &empty_policy_rules(),
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
            &empty_policy_rules(),
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
    async fn allow_without_always_does_not_cascade() {
        // Two pending shell::exec calls in the same session. Resolving
        // the first with allow (always=false) must NOT touch the second.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        for cid in ["tc-1", "tc-2"] {
            let mut rec = build_pending_record(cid, "shell::exec", &json!({}), 1_000, 60_000);
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), json!("s1"));
            bus.set(STATE_SCOPE, &pending_key("s1", cid), rec)
                .await
                .unwrap();
        }
        let rules = empty_policy_rules();
        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        assert!(
            resp.get("cascaded").is_none(),
            "cascaded field must be omitted when always was not set: {resp}"
        );
        let other = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-2"))
            .await
            .unwrap();
        assert_eq!(other["status"], "pending");
        assert_eq!(rules.read().unwrap().len(), 0, "rule must not be pushed");
    }


    #[tokio::test]
    async fn allow_with_always_pushes_rule_and_cascades_same_session_pending() {
        // Three pending calls in session s1: two shell::exec, one
        // shell::fs::write. Resolving the first shell::exec with
        // always=true must:
        //   1. Push an Allow rule for shell::exec
        //   2. Auto-resolve the other shell::exec pending in this session
        //   3. Leave the shell::fs::write pending untouched
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        for (cid, fn_id) in [
            ("tc-1", "shell::exec"),
            ("tc-2", "shell::exec"),
            ("tc-3", "shell::fs::write"),
        ] {
            let mut rec = build_pending_record(cid, fn_id, &json!({}), 1_000, 60_000);
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), json!("s1"));
            bus.set(STATE_SCOPE, &pending_key("s1", cid), rec)
                .await
                .unwrap();
        }
        let rules = empty_policy_rules();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        assert_eq!(
            resp["cascaded"], json!(1),
            "tc-2 should cascade; tc-1 originator excluded; tc-3 not matched"
        );

        // The Allow rule for shell::exec is now in the shared ruleset.
        let pushed = rules.read().unwrap();
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].permission, "shell::exec");
        assert_eq!(pushed[0].action, rules::Action::Allow);
        drop(pushed);

        // Originator and cascaded record both transitioned to executed.
        let r1 = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        let r2 = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-2"))
            .await
            .unwrap();
        let r3 = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-3"))
            .await
            .unwrap();
        assert_eq!(r1["status"], "executed");
        assert_eq!(r2["status"], "executed");
        assert_eq!(
            r3["status"], "pending",
            "non-matching function_id must stay pending: {r3}"
        );

        // Executor was invoked twice: originator + cascaded.
        assert_eq!(exec.calls.lock().unwrap().len(), 2);
    }


    #[tokio::test]
    async fn cascade_does_not_cross_session_boundary() {
        // tc-1 in session s1, tc-2 in session s2 — both shell::exec.
        // Resolving s1/tc-1 with always must not touch s2/tc-2.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        for (session, cid) in [("s1", "tc-1"), ("s2", "tc-2")] {
            let mut rec = build_pending_record(cid, "shell::exec", &json!({}), 1_000, 60_000);
            rec.as_object_mut()
                .unwrap()
                .insert("session_id".into(), json!(session));
            bus.set(STATE_SCOPE, &pending_key(session, cid), rec)
                .await
                .unwrap();
        }
        let rules = empty_policy_rules();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        assert!(
            resp.get("cascaded").is_none() || resp["cascaded"] == json!(0),
            "no record in s1 to cascade onto; tc-2 in s2 must NOT be touched: {resp}"
        );

        let other_session = bus
            .get(STATE_SCOPE, &pending_key("s2", "tc-2"))
            .await
            .unwrap();
        assert_eq!(other_session["status"], "pending");
        assert_eq!(
            exec.calls.lock().unwrap().len(),
            1,
            "only the originator should have been invoked"
        );
    }


    #[tokio::test]
    async fn cascade_skips_originator_record() {
        // Single pending record. always=true must not double-resolve it.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        let mut rec = build_pending_record("tc-1", "shell::exec", &json!({}), 1_000, 60_000);
        rec.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec)
            .await
            .unwrap();
        let rules = empty_policy_rules();

        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        // Originator counts under the existing allow path, not the cascade.
        assert!(resp.get("cascaded").is_none() || resp["cascaded"] == json!(0));
        assert_eq!(exec.calls.lock().unwrap().len(), 1);
    }


    #[tokio::test]
    async fn cascade_skips_already_resolved_records_in_session() {
        // Two records in s1: tc-1 pending, tc-2 already terminal. The
        // cascade must skip tc-2.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        let mut r1 = build_pending_record("tc-1", "shell::exec", &json!({}), 1_000, 60_000);
        r1.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), r1)
            .await
            .unwrap();
        let mut r2 = build_pending_record("tc-2", "shell::exec", &json!({}), 1_000, 60_000);
        r2.as_object_mut()
            .unwrap()
            .insert("session_id".into(), json!("s1"));
        let r2_done = transition_record(&r2, "executed", Some(json!({"ok": true})), None, None);
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), r2_done)
            .await
            .unwrap();

        let rules = empty_policy_rules();
        let resp = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &rules,
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "allow",
                "always": true,
            }),
            1_500,
        )
        .await;
        assert_eq!(resp["ok"], true);
        // tc-2 is terminal — not pending — so cascade skips it.
        assert!(resp.get("cascaded").is_none() || resp["cascaded"] == json!(0));
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
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "denial": {
                    "kind": "user_corrected",
                    "detail": { "feedback": "not authorized" }
                },
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
        assert_eq!(rec["denial"]["kind"], "user_corrected");
        assert_eq!(rec["denial"]["detail"]["feedback"], "not authorized");
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
            &empty_policy_rules(),
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
            &empty_policy_rules(),
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
            &empty_policy_rules(),
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
            &empty_policy_rules(),
            json!({"function_call_id": "tc-1", "session_id": "s1", "decision": "deny"}),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "already_resolved");
    }


    #[tokio::test]
    async fn resolve_deny_without_denial_defaults_to_user_rejected() {
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
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
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
        assert_eq!(stored["denial"]["kind"], "user_rejected");
    }


    #[tokio::test]
    async fn resolve_deny_rejects_malformed_denial() {
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
            &empty_policy_rules(),
            json!({
                "session_id": "s1",
                "function_call_id": "tc-1",
                "decision": "deny",
                "denial": { "kind": "not_a_real_kind" },
            }),
            1_500,
        )
        .await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "bad_denial");
    }


    #[tokio::test]
    async fn handle_lookup_record_rejects_when_only_one_id_is_empty() {
        // mutant L395: `||` → `&&` would let one-empty slip through.
        let bus = InMemoryStateBus::new();
        let v1 = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "", "function_call_id": "c"}),
        )
        .await;
        assert!(v1.is_null());
        let v2 = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "s", "function_call_id": ""}),
        )
        .await;
        assert!(v2.is_null());
    }


    #[tokio::test]
    async fn handle_resolve_rejects_when_only_one_id_is_empty() {
        // mutant L489: same `||` pattern in handle_resolve guard.
        let bus = InMemoryStateBus::new();
        let exec = FakeExecutor::default();
        let r1 = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"session_id": "", "function_call_id": "c", "decision": "allow"}),
            0,
        )
        .await;
        assert_eq!(r1["error"], json!("missing_id"));
        let r2 = handle_resolve(
            &bus,
            &exec,
            STATE_SCOPE,
            &empty_policy_rules(),
            json!({"session_id": "s", "function_call_id": "", "decision": "allow"}),
            0,
        )
        .await;
        assert_eq!(r2["error"], json!("missing_id"));
    }


    #[tokio::test]
    async fn handle_lookup_record_short_circuits_before_bus_get_on_one_empty_id() {
        // mutant L395 — `||` → `&&` would let one-empty slip into bus.get.
        // Seed a record at the address the mutant would compute (pending_key("", "c") = "/c"),
        // so the mutant returns the seeded row while original code stays at Null.
        let bus = InMemoryStateBus::new();
        bus.set(STATE_SCOPE, "/c", json!({"sentinel": "should_not_leak"}))
            .await
            .unwrap();
        let v = handle_lookup_record(
            &bus,
            STATE_SCOPE,
            json!({"session_id": "", "function_call_id": "c"}),
        )
        .await;
        assert!(
            v.is_null(),
            "must short-circuit; the seeded sentinel must not leak through"
        );
    }
