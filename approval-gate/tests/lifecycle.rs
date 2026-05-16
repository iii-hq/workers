//! End-to-end lifecycle tests for the approval-gate simplification (T17).
//!
//! Each test exercises the full intercept → resolve → consume flow against
//! the in-memory `StateBus` + `FunctionExecutor` fakes. These are the
//! integration safety net for the whole refactor — they assert that the
//! pieces snap together correctly without an iii engine.
//!
//! Four flows:
//!   1. Allow path — full Ask → Allow → Executed → consume drains it
//!   2. Deny path — full Ask → Deny(UserCorrected) → consume drains it
//!   3. Timeout path — Pending past expires_at → consume lazy-flips → drains it
//!   4. Cascade path — two Pending rows; allow+always; both end up consumed

mod common;

use approval_gate::record::{Outcome, Record, Status};
use approval_gate::rules::{Action, Rule, Ruleset};
use approval_gate::{
    handle_consume, handle_intercept, handle_resolve, IncomingCall, StateBus, STATE_SCOPE,
};
use common::{FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

fn call(session: &str, cid: &str, fn_id: &str, args: Value) -> IncomingCall {
    IncomingCall {
        session_id: session.into(),
        function_call_id: cid.into(),
        function_id: fn_id.into(),
        args,
        approval_required: Vec::new(),
        event_id: format!("evt-{cid}"),
        reply_stream: format!("rs-{cid}"),
    }
}

fn ruleset_with(rules: Vec<Rule>) -> Arc<RwLock<Ruleset>> {
    Arc::new(RwLock::new(rules))
}

#[tokio::test]
async fn allow_path_end_to_end() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    *exec.response.lock().unwrap() = Some(Ok(json!({ "stdout": "hello\n" })));
    // Empty ruleset → verdict defaults to Ask → intercept writes Pending.
    let policy_rules = ruleset_with(vec![]);

    // 1. Hook fires; gate writes Pending.
    let incoming = call("sess_allow", "tc-1", "shell::exec",
        json!({"command": "echo", "args": ["hello"]}));
    let reply = handle_intercept(
        &bus, STATE_SCOPE, &incoming, &policy_rules.read().unwrap(),
        1_000, 60_000,
    ).await;
    assert_eq!(reply["status"], "pending");

    // 2. Operator resolves allow.
    let payload = json!({
        "session_id": "sess_allow",
        "function_call_id": "tc-1",
        "decision": "allow",
    });
    let resolve_reply = handle_resolve(&bus, &exec, STATE_SCOPE, &policy_rules, payload, 2_000).await;
    assert_eq!(resolve_reply["ok"], true);

    // Executor must have been called exactly once with the original argv.
    let calls = exec.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "shell::exec");
    assert_eq!(calls[0].1["command"], "echo");
    drop(calls);

    // 3. Consume drains the Done row.
    let consume_reply = handle_consume(
        &bus, STATE_SCOPE,
        json!({ "session_id": "sess_allow" }), 3_000,
    ).await;
    assert_eq!(consume_reply["ok"], true);
    let entries = consume_reply["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["outcome"]["kind"], "executed");
    assert_eq!(entries[0]["outcome"]["detail"]["result"]["stdout"], "hello\n");

    // 4. Row gone from state.
    let leftover = bus.list_prefix(STATE_SCOPE, "sess_allow/").await;
    assert!(leftover.is_empty(), "consume must delete drained rows");
}

#[tokio::test]
async fn deny_path_with_user_corrected_feedback_end_to_end() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    let policy_rules = ruleset_with(vec![]);

    let incoming = call("sess_deny", "tc-2", "shell::exec",
        json!({"command": "rm", "args": ["-rf", "/tmp/x"]}));
    handle_intercept(
        &bus, STATE_SCOPE, &incoming, &policy_rules.read().unwrap(),
        1_000, 60_000,
    ).await;

    // Operator denies with a correction message.
    let payload = json!({
        "session_id": "sess_deny",
        "function_call_id": "tc-2",
        "decision": "deny",
        "denial": {
            "kind": "user_corrected",
            "detail": { "feedback": "wrong path, use /tmp/y" },
        },
    });
    let r = handle_resolve(&bus, &exec, STATE_SCOPE, &policy_rules, payload, 2_000).await;
    assert_eq!(r["ok"], true);
    // Shell must NOT have been invoked.
    assert_eq!(exec.calls.lock().unwrap().len(), 0);

    let consume = handle_consume(
        &bus, STATE_SCOPE,
        json!({ "session_id": "sess_deny" }), 3_000,
    ).await;
    let entries = consume["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["outcome"]["kind"], "denied");
    assert_eq!(
        entries[0]["outcome"]["detail"]["denial"]["kind"],
        "user_corrected",
    );
    assert_eq!(
        entries[0]["outcome"]["detail"]["denial"]["detail"]["feedback"],
        "wrong path, use /tmp/y",
    );

    let leftover = bus.list_prefix(STATE_SCOPE, "sess_deny/").await;
    assert!(leftover.is_empty());
}

#[tokio::test]
async fn timeout_path_lazy_flips_on_consume_end_to_end() {
    let bus = InMemoryStateBus::new();
    let policy_rules = ruleset_with(vec![]);

    // Seed a Pending row with a very short timeout.
    let incoming = call("sess_timeout", "tc-3", "shell::exec",
        json!({"command": "ls"}));
    handle_intercept(
        &bus, STATE_SCOPE, &incoming, &policy_rules.read().unwrap(),
        1_000,  // now_ms
        1,      // timeout_ms → expires_at = 1_001
    ).await;

    // Consume at now=2_000, well past expires_at. Lazy flip + return.
    let consume = handle_consume(
        &bus, STATE_SCOPE,
        json!({ "session_id": "sess_timeout" }), 2_000,
    ).await;
    let entries = consume["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["outcome"]["kind"], "timed_out");

    let leftover = bus.list_prefix(STATE_SCOPE, "sess_timeout/").await;
    assert!(leftover.is_empty());
}

#[tokio::test]
async fn cascade_path_end_to_end() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    *exec.response.lock().unwrap() = Some(Ok(json!({ "stdout": "" })));
    let policy_rules = ruleset_with(vec![]);

    // Two pending rows for the SAME argv shape — `cascade_allow_for_session`
    // pushes an exact-pattern Allow rule from the first row's args; the
    // second row should then verdict as Allow and auto-resolve.
    for cid in ["tc-4", "tc-5"] {
        let incoming = call("sess_cascade", cid, "shell::exec",
            json!({"command": "echo", "args": ["go"]}));
        handle_intercept(
            &bus, STATE_SCOPE, &incoming, &policy_rules.read().unwrap(),
            1_000, 60_000,
        ).await;
    }

    // Resolve tc-4 with always:true.
    let payload = json!({
        "session_id": "sess_cascade",
        "function_call_id": "tc-4",
        "decision": "allow",
        "always": true,
    });
    let r = handle_resolve(&bus, &exec, STATE_SCOPE, &policy_rules, payload, 2_000).await;
    assert_eq!(r["ok"], true);
    assert_eq!(r["cascaded"], 1, "one extra row (tc-5) must have auto-resolved");

    // Executor called twice (originator + cascade).
    assert_eq!(exec.calls.lock().unwrap().len(), 2);

    // Both rows in state as Done(Executed). Consume drains them.
    let consume = handle_consume(
        &bus, STATE_SCOPE,
        json!({ "session_id": "sess_cascade" }), 3_000,
    ).await;
    let entries = consume["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for e in entries {
        assert_eq!(e["outcome"]["kind"], "executed");
    }

    // Ruleset gained the runtime Allow rule with the exact pattern.
    let rs = policy_rules.read().unwrap();
    let pushed = rs.last().expect("cascade must push a rule");
    assert_eq!(pushed.action, Action::Allow);
    assert_eq!(pushed.permission, "shell::exec");
    assert_eq!(
        pushed.pattern, "echo go",
        "exact-pattern push (NOT blanket '*') — 'always allow echo go' does not grant rm -rf /",
    );

    let leftover = bus.list_prefix(STATE_SCOPE, "sess_cascade/").await;
    assert!(leftover.is_empty());
}

#[tokio::test]
async fn allow_rule_short_circuits_with_no_state_write() {
    // Bonus: not from the plan, but worth pinning. A Verdict::Allow at
    // intercept time must NOT write a Pending row (no state, no consume).
    let bus = InMemoryStateBus::new();
    let policy_rules = ruleset_with(vec![Rule {
        permission: "shell::exec".into(),
        pattern: "git status*".into(),
        action: Action::Allow,
    }]);

    let incoming = call("sess_pass", "tc-6", "shell::exec",
        json!({"command": "git", "args": ["status"]}));
    let reply = handle_intercept(
        &bus, STATE_SCOPE, &incoming, &policy_rules.read().unwrap(),
        1_000, 60_000,
    ).await;
    assert_eq!(reply["block"], false);

    let leftover = bus.list_prefix(STATE_SCOPE, "sess_pass/").await;
    assert!(leftover.is_empty(), "Allow path must not touch state");
}
