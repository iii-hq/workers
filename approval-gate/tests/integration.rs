//! Engine-backed integration suite — the worker's production surface
//! registered against a real iii engine.

use serde_json::json;

use approval_gate::testkit::{
    call, hook_input, log_snapshot, settle, wait_for, with_stack, BootOpts,
};
use approval_gate::types::PermissionMode;

#[tokio::test(flavor = "multi_thread")]
async fn hold_writes_record_emits_once_and_is_idempotent() {
    with_stack(BootOpts::needs_approval(), |stack| async move {
        let iii = &stack.iii;

        let out = call(
            iii,
            "approval::gate",
            hook_input("s_1", "c_1", "shell::run"),
        )
        .await
        .expect("gate call");
        assert_eq!(out["decision"], json!("hold"));

        let record = call(
            iii,
            "state::get",
            json!({ "scope": "approval_pending", "key": "s_1/c_1" }),
        )
        .await
        .expect("state get");
        assert_eq!(record["function_id"], json!("shell::run"));
        assert_eq!(record["arguments_excerpt"]["api_key"], json!("<redacted>"));
        assert_eq!(record["session_title"], json!("Integration session"));

        let again = call(
            iii,
            "approval::gate",
            hook_input("s_1", "c_1", "shell::run"),
        )
        .await
        .expect("gate call");
        assert_eq!(again["decision"], json!("hold"));

        assert!(
            wait_for(3_000, || !log_snapshot(&stack.created).is_empty()).await,
            "pending_created should reach the bound recorder"
        );
        settle().await;
        assert_eq!(log_snapshot(&stack.created).len(), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_allow_releases_and_deny_delivers_through_the_fake_harness() {
    with_stack(BootOpts::needs_approval(), |stack| async move {
        let iii = &stack.iii;

        for cid in ["c_allow", "c_deny"] {
            let out = call(iii, "approval::gate", hook_input("s_1", cid, "shell::run"))
                .await
                .unwrap();
            assert_eq!(out["decision"], json!("hold"));
        }

        let res = call(
            iii,
            "approval::resolve",
            json!({ "session_id": "s_1", "function_call_id": "c_allow", "decision": "allow" }),
        )
        .await
        .unwrap();
        assert_eq!(res["resolved"], json!(true));

        let res = call(
            iii,
            "approval::resolve",
            json!({
                "session_id": "s_1",
                "function_call_id": "c_deny",
                "decision": "deny",
                "reason": "not on prod"
            }),
        )
        .await
        .unwrap();
        assert_eq!(res["resolved"], json!(true));

        let harness = log_snapshot(&stack.harness_calls);
        assert_eq!(harness.len(), 2);
        assert_eq!(harness[0]["action"], json!("execute"));
        assert_eq!(harness[1]["action"], json!("deliver"));
        assert_eq!(harness[1]["details"]["denied_by"], json!("user"));

        let listed = call(iii, "approval::list-pending", json!({}))
            .await
            .unwrap();
        assert_eq!(listed["pending"].as_array().unwrap().len(), 0);

        assert!(
            wait_for(3_000, || log_snapshot(&stack.resolved).len() >= 2).await,
            "pending_resolved should reach the bound recorder"
        );
        settle().await;
        assert_eq!(log_snapshot(&stack.resolved).len(), 2);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sweep_expires_records_and_emits_timeout_exactly_once() {
    with_stack(BootOpts::needs_approval(), |stack| async move {
        let iii = &stack.iii;

        call(
            iii,
            "state::set",
            json!({ "scope": "approval_pending", "key": "s_9/c_9", "value": {
                "session_id": "s_9",
                "turn_id": "t_9",
                "function_call_id": "c_9",
                "function_id": "shell::run",
                "arguments_excerpt": {},
                "pending_at": 100,
                "expires_at": 200,
                "depth": 0,
            }}),
        )
        .await
        .unwrap();

        let swept = call(iii, "approval::sweep", json!({})).await.unwrap();
        assert_eq!(swept["swept"], json!(1));

        let harness = log_snapshot(&stack.harness_calls);
        assert_eq!(harness.len(), 1);
        assert_eq!(harness[0]["details"]["status"], json!("timeout"));

        let swept = call(iii, "approval::sweep", json!({})).await.unwrap();
        assert_eq!(swept["swept"], json!(0));

        assert!(
            wait_for(3_000, || !log_snapshot(&stack.resolved).is_empty()).await,
            "timeout event should reach the recorder"
        );
        settle().await;
        assert_eq!(log_snapshot(&stack.resolved).len(), 1);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_set_reloads_defaults_reactively() {
    with_stack(BootOpts::needs_approval(), |stack| async move {
        let iii = &stack.iii;

        let out = call(
            iii,
            "approval::gate",
            hook_input("s_2", "c_1", "shell::run"),
        )
        .await
        .unwrap();
        assert_eq!(out["decision"], json!("hold"));

        call(
            iii,
            "configuration::set",
            json!({ "id": "approval-gate", "value": { "default_mode": "full" } }),
        )
        .await
        .expect("configuration set");

        assert!(
            wait_for(5_000, || {
                stack
                    .config
                    .try_read()
                    .map(|cfg| cfg.default_mode == PermissionMode::Full)
                    .unwrap_or(false)
            })
            .await,
            "config should reload reactively"
        );

        let out = call(
            iii,
            "approval::gate",
            hook_input("s_3", "c_2", "shell::run"),
        )
        .await
        .unwrap();
        assert_eq!(out["decision"], json!("continue"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_are_lazily_seeded_against_real_state() {
    with_stack(BootOpts::needs_approval(), |stack| async move {
        let iii = &stack.iii;
        let before = call(
            iii,
            "approval::get-settings",
            json!({ "session_id": "s_lazy" }),
        )
        .await
        .unwrap();
        assert_eq!(before["source"], json!("defaults"));

        call(
            iii,
            "approval::approve-always",
            json!({ "session_id": "s_lazy", "function_id": "shell::run" }),
        )
        .await
        .unwrap();

        let after = call(
            iii,
            "approval::get-settings",
            json!({ "session_id": "s_lazy" }),
        )
        .await
        .unwrap();
        assert_eq!(after["source"], json!("stored"));

        let out = call(
            iii,
            "approval::gate",
            hook_input("s_lazy", "c_1", "shell::run"),
        )
        .await
        .unwrap();
        assert_eq!(out["decision"], json!("continue"));
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn human_only_targets_are_denied_through_the_full_stack() {
    with_stack(BootOpts::allow(), |stack| async move {
        let out = call(
            &stack.iii,
            "approval::gate",
            hook_input("s_1", "c_1", "approval::set-mode"),
        )
        .await
        .unwrap();
        assert_eq!(out["decision"], json!("deny"));
    })
    .await;
}
