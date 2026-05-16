//! handle_intercept — the gate's intercept-time decision path.
//! Covers replay handling, fail-closed on state-write errors, the
//! session_id stamping, and the force_pending classifier branch.

mod common;

use approval_gate::*;
use common::{empty_policy_rules, sample_call, FailingStateBus, FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::Mutex;



    #[tokio::test]
    async fn handle_intercept_returns_pending_envelope_when_call_is_gated() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("pending"));
        assert_eq!(reply["call_id"], json!("tc-1"));
        assert_eq!(reply["function_id"], json!("shell::fs::write"));
        // Pending status is self-describing — no `reason` or `denial` field
        // is emitted while the call is in-flight.
        assert!(reply.get("reason").is_none());
        assert!(reply.get("denial").is_none());
    }


    #[tokio::test]
    async fn handle_intercept_writes_pending_record_to_state() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let key = pending_key(&call.session_id, &call.function_call_id);
        let rec = bus
            .get(STATE_SCOPE, &key)
            .await
            .expect("pending record written");
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["function_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 61_000);
    }


    #[tokio::test]
    async fn handle_intercept_passes_through_when_call_is_not_gated() {
        let bus = InMemoryStateBus::new();
        let mut call = sample_call();
        call.approval_required = vec!["other".into()];
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(false));
        let key = pending_key(&call.session_id, &call.function_call_id);
        assert!(
            bus.get(STATE_SCOPE, &key).await.is_none(),
            "no record written"
        );
    }


    #[tokio::test]
    async fn handle_intercept_force_pending_writes_when_not_on_required_list() {
        let bus = InMemoryStateBus::new();
        let mut call = sample_call();
        call.approval_required = vec!["other".into()];
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, true).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("pending"));
        let key = pending_key(&call.session_id, &call.function_call_id);
        assert!(bus.get(STATE_SCOPE, &key).await.is_some());
    }


    #[tokio::test]
    async fn handle_intercept_fails_closed_on_state_write_error() {
        let bus = FailingStateBus;
        let call = sample_call();
        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(
            reply["block"],
            json!(true),
            "state write failure must NOT fail-open"
        );
        assert_eq!(reply["status"], json!("denied"));
        assert_eq!(reply["denial"]["kind"], json!("state_error"));
        assert_eq!(
            reply["denial"]["detail"]["phase"],
            json!("intercept_write_pending")
        );
        // The underlying error message is present but its exact text is
        // bus-implementation-specific; just check it's non-empty.
        assert!(
            reply["denial"]["detail"]["error"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "state_error detail must include error message: {reply}"
        );
        assert_eq!(reply["function_id"], json!("shell::fs::write"));
    }


    #[tokio::test]
    async fn handle_intercept_stamps_session_id_into_pending_record() {
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        let rec = bus
            .get(
                STATE_SCOPE,
                &pending_key(&call.session_id, &call.function_call_id),
            )
            .await
            .expect("pending record");
        assert_eq!(rec["session_id"], json!(call.session_id));
    }


    // ── Boundary + edge-case tests prompted by cargo-mutants survivors ────
    //
    // Each test corresponds to a mutant the test suite previously didn't
    // catch. Test name → mutated line in src/lib.rs.

    #[tokio::test]
    async fn handle_intercept_replay_of_terminal_record_returns_already_resolved() {
        // mutant L331: replace `==` with `!=` in the replay defense — if
        // flipped, terminal records would be overwritten with fresh pending.
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let key = pending_key(&call.session_id, &call.function_call_id);
        let terminal = transition_record(
            &build_pending_record(
                &call.function_call_id,
                &call.function_id,
                &call.args,
                0,
                60_000,
            ),
            "executed",
            Some(json!({"ok": true})),
            None,
            None,
        );
        bus.set(STATE_SCOPE, &key, terminal).await.unwrap();

        let reply = handle_intercept(&bus, STATE_SCOPE, &call, 1_000, 60_000, false).await;
        assert_eq!(reply["block"], json!(true));
        assert_eq!(reply["status"], json!("executed"));
        // Replay reply: status carries the prior outcome, `replay` discriminator
        // says we're echoing rather than denying afresh, and no `denial` is
        // synthesized (the historical record is the source of truth).
        assert_eq!(reply["replay"], json!("already_resolved"));
        assert!(reply.get("denial").is_none());
        assert!(reply.get("reason").is_none());

        // Crucial: the stored row is still `executed`, not overwritten.
        let stored = bus.get(STATE_SCOPE, &key).await.unwrap();
        assert_eq!(stored["status"], json!("executed"));
        assert_eq!(stored["result"], json!({"ok": true}));
    }


    #[tokio::test]
    async fn handle_intercept_replay_of_pending_record_preserves_expires_at() {
        // mutant L331: same branch, pending side. New pending must not bump
        // the expires_at on the existing row.
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let key = pending_key(&call.session_id, &call.function_call_id);
        let pending = build_pending_record(
            &call.function_call_id,
            &call.function_id,
            &call.args,
            0,
            60_000,
        );
        bus.set(STATE_SCOPE, &key, pending.clone()).await.unwrap();

        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 999_000, 60_000, false).await;
        let stored = bus.get(STATE_SCOPE, &key).await.unwrap();
        assert_eq!(
            stored["expires_at"], pending["expires_at"],
            "replay must not bump expires_at on the live row"
        );
    }


    #[tokio::test]
    async fn handle_intercept_replay_of_approved_record_preserves_state() {
        // mutant L331:42 — replace `==` with `!=` on the "approved" side.
        // The L331:19 mutation is killed by the *_pending_* test above;
        // this one requires an approved record specifically.
        let bus = InMemoryStateBus::new();
        let call = sample_call();
        let key = pending_key(&call.session_id, &call.function_call_id);
        let approved = transition_record(
            &build_pending_record(
                &call.function_call_id,
                &call.function_id,
                &call.args,
                0,
                60_000,
            ),
            "approved",
            None,
            None,
            None,
        );
        bus.set(STATE_SCOPE, &key, approved.clone()).await.unwrap();

        let _ = handle_intercept(&bus, STATE_SCOPE, &call, 999_000, 60_000, false).await;
        let stored = bus.get(STATE_SCOPE, &key).await.unwrap();
        assert_eq!(
            stored["status"],
            json!("approved"),
            "replay of approved row must keep status; mutant would overwrite with pending"
        );
    }
