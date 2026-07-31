//! Cross-worker integration: real `harness` + in-process approval-gate on a
//! live engine. Self-skips when `iii` or the harness binary is unavailable.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use approval_gate::testkit::{
    boot, boot_harness_only, call, engine_bin, engine_test_guard, hook_input, spawn_engine,
    state_get, state_set, BootOpts,
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
    if std::env::var("CARGO_BIN_EXE_harness").is_err() {
        eprintln!("skipping: harness binary not built for integration tests");
        return;
    }
    let _guard = engine_test_guard().await;
    let engine = spawn_engine()
        .await
        .expect("iii engine was provided but failed to start");
    let stack = boot(&engine, opts).await;
    f(stack).await;
}

async fn with_harness_only<F, Fut>(f: F)
where
    F: FnOnce(approval_gate::testkit::HarnessOnlyStack) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if engine_bin().is_none() {
        eprintln!("skipping: no iii engine");
        return;
    }
    if std::env::var("CARGO_BIN_EXE_harness").is_err() {
        eprintln!("skipping: harness binary not built for integration tests");
        return;
    }
    let _guard = engine_test_guard().await;
    let engine = spawn_engine()
        .await
        .expect("iii engine was provided but failed to start");
    let stack = boot_harness_only(&engine).await;
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
            "functions": { "deny": [], "expose": "agent_trigger" },
            "max_validation_retries": 2,
        },
        "calls": calls,
        "created_at": 1,
        "updated_at": 1,
    });
    state_set(&stack.iii, "harness_turn", session_id, record).await;
}

async fn pre_trigger_instance_count(iii: &iii_sdk::IIIClient) -> u64 {
    call(
        iii,
        "engine::triggers::info",
        json!({ "id": "harness::hook::pre-trigger" }),
    )
    .await
    .ok()
    .and_then(|value| value.get("instance_count").and_then(|count| count.as_u64()))
    .unwrap_or(0)
}

async fn wait_for_pre_trigger_binding(iii: &iii_sdk::IIIClient) -> u64 {
    for _ in 0..50 {
        let count = pre_trigger_instance_count(iii).await;
        if count > 0 {
            return count;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    0
}

fn register_send_dependencies(iii: &iii_sdk::IIIClient) {
    iii.register_function(
        "session::ensure",
        RegisterFunction::new_async(|_req: serde_json::Value| async {
            Ok::<_, iii_sdk::errors::Error>(json!({ "created": true }))
        }),
    );
    iii.register_function(
        "session::append",
        RegisterFunction::new_async(|_req: serde_json::Value| async {
            Ok::<_, iii_sdk::errors::Error>(json!({ "entry_id": "e_policy" }))
        }),
    );
}

async fn send_with_functions(
    iii: &iii_sdk::IIIClient,
    session_id: &str,
    functions: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut request = json!({
        "session_id": session_id,
        "message": "exercise dispatch policy",
        "model": "test"
    });
    if let Some(functions) = functions {
        request["options"] = json!({ "functions": functions });
    }
    let sent = call(iii, "harness::send", request).await.expect("send");
    assert_eq!(sent["accepted"], json!(true));
    state_get(iii, "harness_turn", session_id).await
}

async fn trigger_probe(
    iii: &iii_sdk::IIIClient,
    session_id: &str,
    call_id: &str,
) -> serde_json::Value {
    call(
        iii,
        "harness::function::trigger",
        json!({
            "session_id": session_id,
            "call": {
                "id": call_id,
                "function_id": "probe::execute",
                "arguments": { "case": session_id }
            }
        }),
    )
    .await
    .expect("trigger")
}

fn assert_policy_denied(result: &serde_json::Value) {
    assert_eq!(result["is_error"], json!(true));
    assert_eq!(result["details"]["error"], json!("policy_denied"));
    assert!(result.get("pending").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn harness_shipped_default_executes_immediately_without_approval_gate() {
    with_harness_only(|stack| async move {
        let iii = &stack.iii;
        assert_eq!(
            pre_trigger_instance_count(iii).await,
            0,
            "standalone harness must have no approval hook"
        );

        register_send_dependencies(iii);
        iii.register_function(
            "probe::execute",
            RegisterFunction::new_async(|req: serde_json::Value| async move {
                Ok::<_, iii_sdk::errors::Error>(json!({ "executed": true, "payload": req }))
            }),
        );

        let sent = call(
            iii,
            "harness::send",
            json!({
                "session_id": "s_yolo",
                "message": "run without a gate",
                "model": "test"
            }),
        )
        .await
        .expect("send");
        assert_eq!(sent["accepted"], json!(true));

        let turn = state_get(iii, "harness_turn", "s_yolo").await;
        let functions = turn["options"]["functions"]
            .as_object()
            .expect("default function policy persisted");
        assert!(
            !functions.contains_key("allow"),
            "shipped default must be deny-only"
        );
        assert_eq!(functions.get("deny"), Some(&json!([])));

        let executed = call(
            iii,
            "harness::function::trigger",
            json!({
                "session_id": "s_yolo",
                "call": {
                    "id": "c_yolo",
                    "function_id": "probe::execute",
                    "arguments": { "value": 42 }
                }
            }),
        )
        .await
        .expect("trigger");
        assert_eq!(executed["is_error"], json!(false));
        assert_eq!(executed["details"]["executed"], json!(true));
        assert_eq!(executed["details"]["payload"]["value"], json!(42));
        assert!(executed.get("pending").is_none());
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn harness_enforces_the_allow_deny_compatibility_matrix_without_approval_gate() {
    with_harness_only(|stack| async move {
        let iii = &stack.iii;
        assert_eq!(pre_trigger_instance_count(iii).await, 0);
        register_send_dependencies(iii);

        let executions = Arc::new(AtomicUsize::new(0));
        let execution_counter = executions.clone();
        iii.register_function(
            "probe::execute",
            RegisterFunction::new_async(move |req: serde_json::Value| {
                let execution_counter = execution_counter.clone();
                async move {
                    execution_counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, iii_sdk::errors::Error>(json!({ "executed": true, "payload": req }))
                }
            }),
        );

        let cases = [
            ("implicit-default", None, true),
            ("present-empty-policy", Some(json!({})), true),
            ("allow-null", Some(json!({ "allow": null })), true),
            (
                "deny-non-matching",
                Some(json!({ "deny": ["other::*"] })),
                true,
            ),
            (
                "deny-exact",
                Some(json!({ "deny": ["probe::execute"] })),
                false,
            ),
            ("deny-glob", Some(json!({ "deny": ["probe::*"] })), false),
            (
                "legacy-allow-match",
                Some(json!({ "allow": ["probe::execute"] })),
                true,
            ),
            (
                "legacy-allow-miss",
                Some(json!({ "allow": ["other::*"] })),
                false,
            ),
            ("explicit-deny-all", Some(json!({ "allow": [] })), false),
            (
                "deny-overrides-wildcard",
                Some(json!({ "allow": ["*"], "deny": ["probe::*"] })),
                false,
            ),
        ];

        for (index, (name, functions, should_execute)) in cases.into_iter().enumerate() {
            let session_id = format!("s_matrix_{index}");
            let turn = send_with_functions(iii, &session_id, functions).await;
            assert!(
                turn["options"]["functions"].is_object(),
                "{name}: resolved policy must be frozen on the turn"
            );

            let before = executions.load(Ordering::SeqCst);
            let result = trigger_probe(iii, &session_id, &format!("c_matrix_{index}")).await;
            if should_execute {
                assert_eq!(result["is_error"], json!(false), "{name}");
                assert_eq!(result["details"]["executed"], json!(true), "{name}");
                assert_eq!(
                    executions.load(Ordering::SeqCst),
                    before + 1,
                    "{name}: target must execute exactly once"
                );
            } else {
                assert_policy_denied(&result);
                assert_eq!(
                    executions.load(Ordering::SeqCst),
                    before,
                    "{name}: denied target must not execute"
                );
            }
        }

        let no_turn = trigger_probe(iii, "s_missing", "c_missing").await;
        assert_policy_denied(&no_turn);
        assert_eq!(
            executions.load(Ordering::SeqCst),
            5,
            "missing authority must not invoke the target"
        );

        let mut legacy_turn = send_with_functions(iii, "s_legacy", None).await;
        legacy_turn["options"]
            .as_object_mut()
            .expect("turn options")
            .remove("functions");
        state_set(iii, "harness_turn", "s_legacy", legacy_turn).await;
        let legacy = trigger_probe(iii, "s_legacy", "c_legacy").await;
        assert_policy_denied(&legacy);
        assert_eq!(
            executions.load(Ordering::SeqCst),
            5,
            "a persisted turn with no policy must remain fail-closed"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn harness_hot_reload_can_restore_deny_all_for_new_turns() {
    with_harness_only(|stack| async move {
        let iii = &stack.iii;
        register_send_dependencies(iii);

        let executions = Arc::new(AtomicUsize::new(0));
        let execution_counter = executions.clone();
        iii.register_function(
            "probe::execute",
            RegisterFunction::new_async(move |_req: serde_json::Value| {
                let execution_counter = execution_counter.clone();
                async move {
                    execution_counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, iii_sdk::errors::Error>(json!({ "executed": true }))
                }
            }),
        );

        let stored = call(iii, "configuration::get", json!({ "id": "harness" }))
            .await
            .expect("get harness config");
        let mut config = stored["value"].clone();
        config["default_functions"] = serde_json::Value::Null;
        call(
            iii,
            "configuration::set",
            json!({ "id": "harness", "value": config }),
        )
        .await
        .expect("set harness config");

        let mut deny_all_session = None;
        for attempt in 0..50 {
            let session_id = format!("s_hot_reload_{attempt}");
            let turn = send_with_functions(iii, &session_id, None).await;
            if turn["options"].get("functions").is_none() {
                deny_all_session = Some(session_id);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let session_id = deny_all_session.expect("hot-reloaded null default must reach new turns");
        let denied = trigger_probe(iii, &session_id, "c_hot_reload").await;
        assert_policy_denied(&denied);
        assert_eq!(executions.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn harness_inherits_and_freezes_the_prior_turn_policy() {
    with_harness_only(|stack| async move {
        let iii = &stack.iii;
        register_send_dependencies(iii);
        iii.register_function(
            "probe::execute",
            RegisterFunction::new_async(|req: serde_json::Value| async move {
                Ok::<_, iii_sdk::errors::Error>(json!({ "executed": true, "payload": req }))
            }),
        );

        let mut first = send_with_functions(
            iii,
            "s_inherit",
            Some(json!({ "allow": ["probe::execute"] })),
        )
        .await;
        assert_eq!(
            first["options"]["functions"]["allow"],
            json!(["probe::execute"])
        );

        first["status"] = json!("completed");
        state_set(iii, "harness_turn", "s_inherit", first).await;

        let inherited = send_with_functions(iii, "s_inherit", None).await;
        assert_eq!(
            inherited["options"]["functions"]["allow"],
            json!(["probe::execute"]),
            "a new turn on the session must inherit the prior policy"
        );

        let merged = call(
            iii,
            "harness::send",
            json!({
                "session_id": "s_inherit",
                "message": "attempt to widen an active turn",
                "model": "test",
                "options": {
                    "functions": {
                        "deny": []
                    }
                }
            }),
        )
        .await
        .expect("steer");
        assert_eq!(merged["accepted"], json!(true));
        assert!(
            merged["merged"] == json!(true) || merged["queued"] == json!(true),
            "an active turn must merge or queue steering"
        );

        let frozen = state_get(iii, "harness_turn", "s_inherit").await;
        assert_eq!(
            frozen["options"]["functions"]["allow"],
            json!(["probe::execute"]),
            "steering must not widen the active turn's frozen policy"
        );
    })
    .await;
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
                "functions": { "deny": [], "expose": "agent_trigger" },
                "max_validation_retries": 2,
            },
            "calls": {},
            "created_at": 1,
            "updated_at": 1,
        });
        state_set(iii, "harness_turn", "s_1", turn).await;
        assert!(
            wait_for_pre_trigger_binding(iii).await > 0,
            "approval-gate pre_trigger hook must be bound"
        );

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
async fn structural_deny_short_circuits_the_approval_gate() {
    with_harness_stack(
        BootOpts::needs_approval_with_harness(),
        |stack| async move {
            let iii = &stack.iii;
            let executions = Arc::new(AtomicUsize::new(0));
            let execution_counter = executions.clone();
            iii.register_function(
                "probe::execute",
                RegisterFunction::new_async(move |_req: serde_json::Value| {
                    let execution_counter = execution_counter.clone();
                    async move {
                        execution_counter.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, iii_sdk::errors::Error>(json!({ "executed": true }))
                    }
                }),
            );

            let turn = json!({
                "turn_id": "t_structural_deny",
                "session_id": "s_structural_deny",
                "status": "running",
                "step": 1,
                "turn_count": 0,
                "depth": 0,
                "abort": false,
                "options": {
                    "model": "test",
                    "max_turns": 8,
                    "output": { "type": "text" },
                    "functions": {
                        "deny": ["probe::*"],
                        "expose": "agent_trigger"
                    },
                    "max_validation_retries": 2,
                },
                "calls": {},
                "created_at": 1,
                "updated_at": 1,
            });
            state_set(iii, "harness_turn", "s_structural_deny", turn).await;
            assert!(wait_for_pre_trigger_binding(iii).await > 0);

            let denied = trigger_probe(iii, "s_structural_deny", "c_structural_deny").await;
            assert_policy_denied(&denied);
            assert_eq!(executions.load(Ordering::SeqCst), 0);

            let pending = call(iii, "approval::list-pending", json!({}))
                .await
                .expect("list pending");
            assert_eq!(
                pending["pending"].as_array().map(Vec::len),
                Some(0),
                "a structural denial must never reach the approval inbox"
            );
        },
    )
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
                    "functions": { "deny": [], "expose": "agent_trigger" },
                    "max_validation_retries": 2,
                },
                "calls": {
                    "c_deny": {
                        "state": "pending",
                        "function_id": "shell::run",
                        "held_by": "approval::gate",
                    }
                },
                "created_at": 1,
                "updated_at": 1,
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
