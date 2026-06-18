//! Cross-worker integration: real `harness` + in-process approval-gate on a
//! live engine. Self-skips when `iii` or the harness binary is unavailable.

use std::collections::BTreeMap;

use approval_gate::testkit::{
    boot, call, engine_bin, hook_input, spawn_engine, state_get, state_set, BootOpts,
};
use iii_sdk::RegisterFunction;
use serde_json::json;

async fn with_harness_stack<F, Fut>(opts: BootOpts, f: F)
where
    F: FnOnce(approval_gate::testkit::TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if engine_bin().is_none() {
        eprintln!("skipping: no iii engine");
        return;
    }
    let Some(engine) = spawn_engine().await else {
        eprintln!("skipping: failed to spawn engine");
        return;
    };
    if std::env::var("CARGO_BIN_EXE_harness").is_err() {
        eprintln!("skipping: harness binary not built for integration tests");
        return;
    }
    let stack = boot(&engine, opts).await;
    f(stack).await;
}

async fn seed_held_turn(
    stack: &approval_gate::testkit::TestStack,
    session_id: &str,
    call_id: &str,
) {
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
    let record = json!({
        "turn_id": "t_1",
        "session_id": session_id,
        "status": "awaiting_functions",
        "step": 1,
        "turn_count": 0,
        "depth": 0,
        "abort": false,
        "options": {
            "model": "test",
            "max_turns": 8,
            "output": { "type": "text" },
            "functions": { "allow": ["*"], "expose": "agent_trigger" },
            "max_validation_retries": 2,
        },
        "calls": calls,
    });
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
async fn harness_pre_trigger_hold_then_resolve_allow() {
    with_harness_stack(BootOpts::needs_approval_with_harness(), |stack| async move {
        let iii = &stack.iii;

        iii.register_function(
            "shell::run",
            RegisterFunction::new_async(|_req: serde_json::Value| async {
                Ok::<_, iii_sdk::IIIError>(json!({ "ok": true }))
            }),
        );

        let turn = json!({
            "turn_id": "t_1",
            "session_id": "s_1",
            "status": "running",
            "step": 1,
            "turn_count": 0,
            "depth": 0,
            "abort": false,
            "options": {
                "model": "test",
                "max_turns": 8,
                "output": { "type": "text" },
                "functions": { "allow": ["*"], "expose": "agent_trigger" },
                "max_validation_retries": 2,
            },
            "calls": {},
        });
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
            "harness::function::trigger",
            json!({
                "session_id": "s_1",
                "call": { "id": "c_1", "function_id": "shell::run", "arguments": { "cmd": "echo hi" } }
            }),
        )
        .await
        .expect("trigger");
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

            call(
                iii,
                "approval::gate",
                hook_input("s_2", "c_deny", "shell::run"),
            )
            .await
            .expect("gate hold");

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

            let turn = json!({
                "turn_id": "t_1",
                "session_id": "s_2",
                "status": "awaiting_functions",
                "step": 1,
                "turn_count": 0,
                "depth": 0,
                "abort": false,
                "options": {
                    "model": "test",
                    "max_turns": 8,
                    "output": { "type": "text" },
                    "functions": { "allow": ["*"], "expose": "agent_trigger" },
                    "max_validation_retries": 2,
                },
                "calls": {
                    "c_deny": {
                        "state": "pending",
                        "function_id": "shell::run",
                        "held_by": "approval::gate",
                    }
                },
            });
            state_set(iii, "harness_turn", "s_2", turn).await;

            let deliver = call(
                iii,
                "harness::function::resolve",
                json!({
                    "session_id": "s_2",
                    "turn_id": "t_1",
                    "function_call_id": "c_deny",
                    "action": "deliver",
                    "is_error": true,
                    "content": [{ "type": "text", "text": "denied" }],
                    "details": { "status": "denied", "denied_by": "user" },
                }),
            )
            .await
            .expect("harness resolve deliver");
            assert_eq!(deliver["resolved"], json!(true));
        },
    )
    .await;
}
