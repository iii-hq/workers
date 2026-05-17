//! Backend-only integration coverage for the approval path.
//!
//! These tests intentionally do not boot an agent loop or the web UI. They
//! connect approval-gate's public handlers to the real `shell::exec` handler
//! through the `FunctionExecutor` seam, then drain the resolved record through
//! `approval::consume`.

mod common;

use std::sync::{Arc, RwLock};

use approval_gate::{
    handle_consume, handle_intercept, handle_resolve, FunctionExecutor, IncomingCall, LayeredRules,
    STATE_SCOPE,
};
use common::InMemoryStateBus;
use serde_json::{json, Value};
use shell::config::ShellConfig;
use shell::functions;
use shell::functions::types::ExecRequest;

struct ShellExec {
    cfg: Arc<ShellConfig>,
}

#[async_trait::async_trait]
impl FunctionExecutor for ShellExec {
    async fn invoke(
        &self,
        function_id: &str,
        args: Value,
        _function_call_id: &str,
        _session_id: &str,
    ) -> Result<Value, String> {
        if function_id != "shell::exec" {
            return Err(format!("unsupported function: {function_id}"));
        }

        let req: ExecRequest = serde_json::from_value(args).map_err(|e| e.to_string())?;
        let resp = functions::exec::handle(
            self.cfg.clone(),
            iii_sdk::III::new("ws://stub-not-connected:0"),
            req,
        )
        .await?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    }
}

fn shell_cfg(allow: &[&str], deny: &[&str]) -> Arc<ShellConfig> {
    let mut cfg = ShellConfig {
        max_timeout_ms: 2_000,
        default_timeout_ms: 2_000,
        max_output_bytes: 4_096,
        allowlist: allow.iter().map(|s| s.to_string()).collect(),
        denylist_patterns: deny.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    cfg.compile_denylist().expect("test denylist compiles");
    Arc::new(cfg)
}

fn call(session: &str, cid: &str, args: Value) -> IncomingCall {
    IncomingCall {
        session_id: session.into(),
        function_call_id: cid.into(),
        function_id: "shell::exec".into(),
        args,
        approval_required: Vec::new(),
        event_id: format!("evt-{cid}"),
        reply_stream: format!("rs-{cid}"),
    }
}

#[tokio::test]
async fn approved_call_executes_real_shell_handler_and_consumes_result() {
    let bus = InMemoryStateBus::new();
    let exec = ShellExec {
        cfg: shell_cfg(&["echo"], &[]),
    };
    let policy_rules = RwLock::new(LayeredRules::default());

    let incoming = call(
        "sess-backend-ok",
        "tc-ok",
        json!({"command": "echo", "args": ["approval-backend-ok"]}),
    );
    let pending = handle_intercept(
        &bus,
        STATE_SCOPE,
        &incoming,
        &policy_rules.read().unwrap().snapshot_for("sess-backend-ok"),
        1_000,
        60_000,
    )
    .await;
    assert_eq!(pending["status"], "pending");

    let resolved = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &policy_rules,
        json!({
            "session_id": "sess-backend-ok",
            "function_call_id": "tc-ok",
            "decision": "allow",
        }),
        2_000,
    )
    .await;
    assert_eq!(resolved["ok"], true);

    let consumed = handle_consume(
        &bus,
        STATE_SCOPE,
        json!({"session_id": "sess-backend-ok"}),
        3_000,
    )
    .await;
    let entries = consumed["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["function_call_id"], "tc-ok");
    assert_eq!(entries[0]["outcome"]["kind"], "executed");
    assert_eq!(
        entries[0]["outcome"]["detail"]["result"]["stdout"],
        "approval-backend-ok\n",
    );
}

#[tokio::test]
async fn approved_call_records_failed_outcome_when_shell_policy_rejects() {
    let bus = InMemoryStateBus::new();
    let exec = ShellExec {
        cfg: shell_cfg(&["echo"], &["^echo blocked"]),
    };
    let policy_rules = RwLock::new(LayeredRules::default());

    let incoming = call(
        "sess-backend-denied-by-shell",
        "tc-denied-by-shell",
        json!({"command": "echo", "args": ["blocked"]}),
    );
    let pending = handle_intercept(
        &bus,
        STATE_SCOPE,
        &incoming,
        &policy_rules
            .read()
            .unwrap()
            .snapshot_for("sess-backend-denied-by-shell"),
        1_000,
        60_000,
    )
    .await;
    assert_eq!(pending["status"], "pending");

    let resolved = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &policy_rules,
        json!({
            "session_id": "sess-backend-denied-by-shell",
            "function_call_id": "tc-denied-by-shell",
            "decision": "allow",
        }),
        2_000,
    )
    .await;
    assert_eq!(
        resolved["ok"], true,
        "approval::resolve records executor failures as resolved failed outcomes",
    );

    let consumed = handle_consume(
        &bus,
        STATE_SCOPE,
        json!({"session_id": "sess-backend-denied-by-shell"}),
        3_000,
    )
    .await;
    let entries = consumed["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["outcome"]["kind"], "failed");
    assert!(
        entries[0]["outcome"]["detail"]["error"]
            .as_str()
            .unwrap_or("")
            .contains("command matches denylist"),
        "unexpected failure entry: {}",
        entries[0],
    );
}
