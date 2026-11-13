//! Cross-worker integration: real `harness` + in-process approval-gate on a
//! live engine. Self-skips when the engine, the state worker or the harness
//! binary (`CARGO_BIN_EXE_harness`) is unavailable.

use std::collections::BTreeMap;

use approval_gate::testkit::{call, hook_input, state_get, state_set, with_stack, BootOpts};
use harness::types::TurnRecord;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

async fn with_harness_stack<F, Fut>(opts: BootOpts, f: F)
where
    F: FnOnce(approval_gate::testkit::TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    with_stack(opts, f).await;
}

/// A `harness_turn` record as the harness stores it, checked against the
/// harness's own `TurnRecord` before a test writes it.
fn turn_record(session_id: &str, status: &str, calls: Value) -> Value {
    let record = json!({
        "turn_id": "t_1",
        "session_id": session_id,
        "status": status,
        "step": 1,
        "turn_count": 0,
        "depth": 0,
        "abort": false,
        "options": {
            "model": "test",
            "max_turns": 8,
            "output": { "type": "text" },
            "functions": { "allow": ["*"], "expose": "agent_call" },
            "max_validation_retries": 2,
        },
        "calls": calls,
        "created_at": 1,
        "updated_at": 1,
    });
    if let Err(e) = serde_json::from_value::<TurnRecord>(record.clone()) {
        panic!("the harness_turn seed is not a harness TurnRecord: {e}");
    }
    record
}

async fn seed_held_turn(
    stack: &approval_gate::testkit::TestStack,
    session_id: &str,
    call_id: &str,
) {
    call(
        &stack.iii,
        "session::ensure",
        json!({"session_id":session_id}),
    )
    .await
    .expect("ensure session");
    let mut calls = BTreeMap::new();
    calls.insert(
        call_id.to_string(),
        json!({
            "state": "pending",
            "function_id": "shell::run",
            "held_by": "approval::gate",
            "pending_timeout_ms": null,
            "pending_at": 1,
        }),
    );
    let record = turn_record(session_id, "awaiting_functions", json!(calls));
    state_set(&stack.iii, "harness_turn", session_id, record).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn harness_held_call_survives_sweep_pending() {
    with_harness_stack(
        BootOpts::needs_approval_with_harness(),
        |stack| async move {
            seed_held_turn(&stack, "s_hold", "c_hold").await;

            let swept = call(&stack.iii, "harness::sweep-pending", json!({}))
                .await
                .expect("sweep");
            assert_eq!(swept["resolved"], json!(0));

            let turn = state_get(&stack.iii, "harness_turn", "s_hold").await;
            assert_eq!(turn["calls"]["c_hold"]["state"], json!("pending"));
        },
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn harness_pre_call_hold_then_resolve_allow() {
    with_harness_stack(BootOpts::needs_approval_with_harness(), |stack| async move {
        let iii = &stack.iii;
        call(iii, "session::ensure", json!({"session_id":"s_1"}))
            .await
            .expect("ensure session");

        iii.register_function(
            "shell::run",
            RegisterFunction::new_async(|_req: serde_json::Value| async {
                Ok::<_, iii_sdk::errors::Error>(json!({ "ok": true }))
            }),
        );

        let turn = turn_record("s_1", "running", json!({}));
        state_set(iii, "harness_turn", "s_1", turn).await;

        let gate = call(
            iii,
            "approval::gate",
            hook_input("s_1", "c_1", "shell::run"),
        )
        .await
        .expect("gate");
        assert_eq!(gate["decision"], json!("hold"));

        let pending = call(
            iii,
            "harness::function::call",
            json!({
                "session_id": "s_1",
                "call": { "id": "c_1", "function_id": "shell::run", "arguments": { "cmd": "echo hi" } }
            }),
        )
        .await
        .expect("call");
        assert_eq!(pending["pending"], json!(true));
        assert!(pending.get("pending_timeout_ms").is_none());

        let res = call(
            iii,
            "approval::resolve",
            json!({ "session_id": "s_1", "function_call_id": "c_1", "decision": "allow" }),
        )
        .await
        .expect("resolve");
        assert_eq!(res["resolved"], json!(true));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_deny_delivers_is_error_through_harness() {
    with_harness_stack(
        BootOpts::needs_approval_with_harness(),
        |stack| async move {
            let iii = &stack.iii;
            // The stack runs the real session-manager: the harness appends there.
            call(iii, "session::ensure", json!({"session_id":"s_2"}))
                .await
                .expect("ensure session");

            // The turn waits on the call the gate holds and on one more held
            // call, so delivering the denial leaves it awaiting functions.
            let turn = turn_record(
                "s_2",
                "awaiting_functions",
                json!({
                    "c_deny": {
                        "state": "pending",
                        "function_id": "shell::run",
                        "held_by": "approval::gate",
                    },
                    "c_other": {
                        "state": "pending",
                        "function_id": "shell::run",
                        "held_by": "approval::gate",
                    },
                }),
            );
            state_set(iii, "harness_turn", "s_2", turn).await;

            let gate = call(
                iii,
                "approval::gate",
                hook_input("s_2", "c_deny", "shell::run"),
            )
            .await
            .expect("gate hold");
            assert_eq!(gate["decision"], json!("hold"));

            let res = call(
                iii,
                "approval::resolve",
                json!({
                    "session_id": "s_2",
                    "function_call_id": "c_deny",
                    "decision": "deny",
                    "reason": "nope"
                }),
            )
            .await
            .expect("resolve");
            assert_eq!(res["resolved"], json!(true));
            assert_eq!(res["turn_resumed"], json!(false));

            // The harness wrote the denial into the transcript as an error result.
            let transcript = call(iii, "session::messages", json!({"session_id":"s_2"}))
                .await
                .expect("session messages");
            let appended = transcript["messages"].as_array().expect("messages");
            assert_eq!(appended.len(), 1, "one appended result: {appended:?}");
            let message = &appended[0]["message"];
            assert_eq!(message["role"], json!("function_result"));
            assert_eq!(message["function_call_id"], json!("c_deny"));
            assert_eq!(message["is_error"], json!(true));

            let turn = state_get(iii, "harness_turn", "s_2").await;
            assert_eq!(turn["calls"]["c_deny"]["state"], json!("done"));
            assert_eq!(turn["calls"]["c_other"]["state"], json!("pending"));
        },
    )
    .await;
}
