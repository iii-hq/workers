//! Cross-worker integration: real `harness` + in-process approval-gate on a
//! live engine with the `queue` worker the harness needs. Self-skips only
//! when no `iii` binary exists; the harness binary is `III_HARNESS_BIN`, or
//! built from `../harness` by the testkit.

use std::collections::BTreeMap;

use approval_gate::testkit::{
    call, hook_input, log_push, log_snapshot, state_get, state_set, with_stack_using, BootOpts,
    CallLog, EngineOpts,
};
use iii_sdk::RegisterFunction;
use serde_json::json;

async fn with_harness_stack<F, Fut>(opts: BootOpts, f: F)
where
    F: FnOnce(approval_gate::testkit::TestStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    with_stack_using(EngineOpts::with_queue(), opts, f).await;
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
        // Required by harness's TurnRecord (no serde default).
        "created_at": 1,
        "updated_at": 1,
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
                Ok::<_, iii_sdk::errors::Error>(json!({ "ok": true }))
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
            // Required by harness's TurnRecord (no serde default).
            "created_at": 1,
            "updated_at": 1,
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

            // The harness appends the delivered result to the session
            // transcript: a fake session-manager records it.
            let appended: CallLog = CallLog::default();
            {
                let appended = appended.clone();
                iii.register_function(
                    "session::append",
                    RegisterFunction::new_async(move |req: serde_json::Value| {
                        let appended = appended.clone();
                        async move {
                            let entry_id = req
                                .get("entry_id")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("e_1")
                                .to_string();
                            log_push(&appended, req);
                            Ok::<_, iii_sdk::errors::Error>(json!({ "entry_id": entry_id }))
                        }
                    }),
                );
            }

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
                // Required by harness's TurnRecord (no serde default).
                "created_at": 1,
                "updated_at": 1,
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
            assert!(
                log_snapshot(&appended)
                    .iter()
                    .any(|entry| entry["session_id"] == json!("s_2")),
                "the denied result must reach the session transcript"
            );
        },
    )
    .await;
}
