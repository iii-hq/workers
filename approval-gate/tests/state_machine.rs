//! Property-based state-machine invariants. Drives the gate through
//! random sequences of (intercept, resolve, sweep, ack, ...) ops and
//! asserts the four invariants documented in the test body.


mod common;

use approval_gate::*;
use common::{empty_policy_rules, FakeExecutor, InMemoryStateBus};
use proptest::prelude::*;
use serde_json::{json, Value};



#[derive(Debug, Clone)]
enum Op {
    InterceptRequired,
    InterceptNotRequired,
    ResolveAllow,
    ResolveDeny,
    AdvanceClockAndLazyFlip,
    SweepSession,
    AckDelivered,
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::InterceptRequired),
        Just(Op::InterceptNotRequired),
        Just(Op::ResolveAllow),
        Just(Op::ResolveDeny),
        Just(Op::AdvanceClockAndLazyFlip),
        Just(Op::SweepSession),
        Just(Op::AckDelivered),
    ]
}

fn make_call(approval_required_self: bool) -> IncomingCall {
    IncomingCall {
        session_id: "s".into(),
        function_call_id: "c".into(),
        function_id: "test::write".into(),
        args: json!({}),
        approval_required: if approval_required_self {
            vec!["test::write".into()]
        } else {
            vec!["other::fn".into()]
        },
        event_id: "e".into(),
        reply_stream: "r".into(),
    }
}



    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            .. ProptestConfig::default()
        })]

        #[test]
        fn state_machine_invariants(ops in prop::collection::vec(arb_op(), 1..30)) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");

            rt.block_on(async {
                let bus = InMemoryStateBus::new();
                let exec = FakeExecutor::default();
                let session_id = "s";
                let call_id = "c";
                let timeout_ms: u64 = 60_000;
                let mut now_ms: u64 = 1_000;

                let mut ever_terminal = false;
                let mut last_delivered: Option<String> = None;

                for op in &ops {
                    match op {
                        Op::InterceptRequired => {
                            let call = make_call(true);
                            let _ = handle_intercept(&bus, STATE_SCOPE, &call, now_ms, timeout_ms, false).await;
                        }
                        Op::InterceptNotRequired => {
                            let call = make_call(false);
                            let _ = handle_intercept(&bus, STATE_SCOPE, &call, now_ms, timeout_ms, false).await;
                        }
                        Op::ResolveAllow => {
                            let _ = handle_resolve(
                                &bus,
                                &exec,
                                STATE_SCOPE,
                                &empty_policy_rules(),
                                json!({
                                    "session_id": session_id,
                                    "function_call_id": call_id,
                                    "decision": "allow",
                                }),
                                now_ms,
                            )
                            .await;
                        }
                        Op::ResolveDeny => {
                            let _ = handle_resolve(
                                &bus,
                                &exec,
                                STATE_SCOPE,
                                &empty_policy_rules(),
                                json!({
                                    "session_id": session_id,
                                    "function_call_id": call_id,
                                    "decision": "deny",
                                }),
                                now_ms,
                            )
                            .await;
                        }
                        Op::AdvanceClockAndLazyFlip => {
                            now_ms = now_ms.saturating_add(timeout_ms + 1);
                            let _ = handle_list_undelivered(
                                &bus, STATE_SCOPE,
                                json!({ "session_id": session_id }),
                                now_ms,
                            ).await;
                        }
                        Op::SweepSession => {
                            let _ = handle_sweep_session(
                                &bus, STATE_SCOPE,
                                json!({ "session_id": session_id }),
                            ).await;
                        }
                        Op::AckDelivered => {
                            let _ = handle_ack_delivered(
                                &bus, STATE_SCOPE,
                                json!({
                                    "session_id": session_id,
                                    "turn_id": format!("turn-{now_ms}"),
                                    "call_ids": [call_id],
                                }),
                            ).await;
                        }
                    }

                    // Assert invariants on whatever the record currently is.
                    let key = pending_key(session_id, call_id);
                    let Some(rec) = bus.get(STATE_SCOPE, &key).await else {
                        // No record yet (e.g. only InterceptNotRequired so far). Skip.
                        continue;
                    };

                    // I1: legal status
                    let status = rec.get("status").and_then(Value::as_str).unwrap_or("");
                    assert!(
                        matches!(
                            status,
                            "pending" | "approved" | "executed" | "failed" | "denied" | "timed_out"
                        ),
                        "I1 violated: illegal status {status:?} after ops {ops:?}; record={rec:?}"
                    );

                    // I2: no reverting terminal → pending
                    if matches!(status, "executed" | "failed" | "denied" | "timed_out") {
                        ever_terminal = true;
                    }
                    if ever_terminal {
                        assert!(
                            status != "pending",
                            "I2 violated: reverted to pending after terminal; ops={ops:?}; record={rec:?}"
                        );
                    }

                    // I3: pending records always have expires_at: u64
                    if status == "pending" {
                        let exp = rec.get("expires_at").and_then(Value::as_u64);
                        assert!(
                            exp.is_some(),
                            "I3 violated: pending record missing expires_at; ops={ops:?}; record={rec:?}"
                        );
                    }

                    // I4: delivered_in_turn_id is monotonic — once set non-null, never unset / never replaced
                    let cur_delivered = rec
                        .get("delivered_in_turn_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(prev) = &last_delivered {
                        match &cur_delivered {
                            Some(cur) => {
                                assert_eq!(
                                    cur, prev,
                                    "I4 violated: delivered_in_turn_id replaced {prev:?} → {cur:?}; ops={ops:?}"
                                );
                            }
                            None => {
                                panic!(
                                    "I4 violated: delivered_in_turn_id unset after being {prev:?}; ops={ops:?}; record={rec:?}"
                                );
                            }
                        }
                    }
                    if cur_delivered.is_some() {
                        last_delivered = cur_delivered;
                    }
                }
            });
        }
    }
