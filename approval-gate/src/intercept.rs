//! Intercept decision flow.
//!
//! One async entry point: [`handle_intercept`]. The classifier surface and
//! per-function `InterceptorRule` flow are gone — the layered rules engine
//! (`crate::rules`) is the only policy decision. `handle_intercept` reads
//! the verdict via [`crate::verdict_for`] and writes a `Pending` row when
//! the verdict is `Ask`. Allow/Deny verdicts return synchronous replies
//! and never touch state.
//!
//! Replay defense recognises all three persisted states:
//! - `Pending` → reply `{replay:"in_flight", status:"pending"}`
//! - `InFlight` → reply `{replay:"in_flight", status:"in_flight"}`
//! - `Done` → reply `{replay:"already_resolved", status:"done"}`
//!
//! None of these overwrite the existing row.

use serde_json::{json, Value};

use crate::record::{Record, Status};
use crate::rules::RulesSnapshot;
use crate::state::StateBus;
use crate::wire::{pending_key, Denial, IncomingCall};

/// Subscriber-side entry point. Decides via `verdict_for` (rules layer);
/// on Ask, persists a Pending record. State-write failure fails closed
/// with `Denial::StateError` so a transient kv outage cannot silently
/// bypass an approval check.
pub async fn handle_intercept(
    bus: &dyn StateBus,
    state_scope: &str,
    call: &IncomingCall,
    rules: &RulesSnapshot,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    // 1. Rules pre-check.
    match crate::verdict_for(&call.function_id, &call.args, rules) {
        crate::Verdict::Allow { rule } => {
            let mut reply = json!({ "block": false });
            if let Some(rule) = rule {
                reply["rule"] = serde_json::to_value(&rule).unwrap_or(Value::Null);
                if let Some(reason) = &rule.reason {
                    reply["reason"] = json!(reason);
                }
            }
            return reply;
        }
        crate::Verdict::Deny { denial, rule } => {
            let mut reply = json!({
                "block": true,
                "status": "denied",
                "denial": denial,
                "call_id": call.function_call_id,
                "function_id": call.function_id,
            });
            if let Some(rule) = rule {
                reply["rule"] = serde_json::to_value(&rule).unwrap_or(Value::Null);
                if let Some(reason) = &rule.reason {
                    reply["reason"] = json!(reason);
                }
            }
            return reply;
        }
        crate::Verdict::Ask { rule } => {
            let key = pending_key(&call.session_id, &call.function_call_id);
            return ask_with_rule(bus, state_scope, call, key, rule, now_ms, timeout_ms).await;
        }
    }
}

async fn ask_with_rule(
    bus: &dyn StateBus,
    state_scope: &str,
    call: &IncomingCall,
    key: String,
    rule: Option<crate::rules::RuleMatch>,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    // 2. Replay defense — never overwrite an existing row.
    if let Some(existing_raw) = bus.get(state_scope, &key).await {
        if let Some(existing) = Record::from_value(existing_raw) {
            return match existing.status {
                Status::Done => json!({
                    "block":       true,
                    "status":      "done",
                    "replay":      "already_resolved",
                    "call_id":     call.function_call_id,
                    "function_id": call.function_id,
                }),
                Status::Pending => json!({
                    "block":       true,
                    "status":      "pending",
                    "replay":      "in_flight",
                    "call_id":     call.function_call_id,
                    "function_id": call.function_id,
                }),
                Status::InFlight => json!({
                    "block":       true,
                    "status":      "in_flight",
                    "replay":      "in_flight",
                    "call_id":     call.function_call_id,
                    "function_id": call.function_id,
                }),
            };
        }
        // Malformed row → fall through and overwrite (defensive).
    }

    // 3. Fresh Pending write.
    let record = Record::pending_with_rule(
        call.function_call_id.clone(),
        call.function_id.clone(),
        call.args.clone(),
        call.session_id.clone(),
        now_ms,
        timeout_ms,
        rule.clone(),
    );
    if let Err(err) = bus.set(state_scope, &key, record.to_value()).await {
        tracing::error!(
            "approval-gate: failed to write pending record for {}/{}: {err} — failing closed",
            call.session_id,
            call.function_call_id,
        );
        let denial = Denial::StateError {
            phase: "intercept_write_pending".to_string(),
            error: err.to_string(),
        };
        return json!({
            "block":       true,
            "denial":      denial,
            "status":      "denied",
            "call_id":     call.function_call_id,
            "function_id": call.function_id,
        });
    }
    let mut reply = json!({
        "block":       true,
        "status":      "pending",
        "call_id":     call.function_call_id,
        "function_id": call.function_id,
    });
    if let Some(rule) = rule {
        reply["rule"] = serde_json::to_value(&rule).unwrap_or(Value::Null);
        if let Some(reason) = &rule.reason {
            reply["reason"] = json!(reason);
        }
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Outcome, Record};
    use crate::rules::{Action, Rule, RulesSnapshot, Ruleset};
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct InMemBus {
        rows: Mutex<std::collections::BTreeMap<(String, String), Value>>,
    }
    #[async_trait::async_trait]
    impl StateBus for InMemBus {
        async fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), iii_sdk::IIIError> {
            self.rows
                .lock()
                .unwrap()
                .insert((scope.into(), key.into()), value);
            Ok(())
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.rows
                .lock()
                .unwrap()
                .get(&(scope.into(), key.into()))
                .cloned()
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .filter(|((s, k), _)| s == scope && k.starts_with(prefix))
                .map(|(_, v)| v.clone())
                .collect()
        }
        async fn delete(&self, scope: &str, key: &str) -> Result<bool, iii_sdk::IIIError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .remove(&(scope.into(), key.into()))
                .is_some())
        }
    }

    fn call(fc_id: &str, fn_id: &str, args: Value) -> IncomingCall {
        IncomingCall {
            session_id: "sess_a".into(),
            function_call_id: fc_id.into(),
            function_id: fn_id.into(),
            args,
            approval_required: Vec::new(),
            event_id: "evt-1".into(),
            reply_stream: "hk-1".into(),
        }
    }

    fn policy(rules: Ruleset) -> RulesSnapshot {
        RulesSnapshot {
            global: Some(rules),
            session: Vec::new(),
        }
    }

    fn r(permission: &str, pattern: &str, action: Action) -> Rule {
        Rule {
            permission: permission.to_string(),
            pattern: pattern.to_string(),
            action,
            reason: None,
        }
    }

    #[tokio::test]
    async fn allow_rule_returns_block_false_no_state_write() {
        let bus = InMemBus::default();
        let rs = policy(vec![r("shell::exec", "git status*", Action::Allow)]);
        let c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "git", "args": ["status"]}),
        );
        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["block"], false);
        assert!(bus.list_prefix("approvals", "sess_a/").await.is_empty());
    }

    #[tokio::test]
    async fn rules_are_authoritative_over_approval_required_list() {
        let bus = InMemBus::default();
        let rs = policy(vec![r("shell::exec", "git status*", Action::Allow)]);
        let mut c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "git", "args": ["status"]}),
        );
        c.approval_required = vec!["shell::exec".into()];

        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;

        assert_eq!(
            reply["block"], false,
            "approval_required is legacy payload context; policy decisions come from rules",
        );
        assert!(bus.list_prefix("approvals", "sess_a/").await.is_empty());
    }

    #[tokio::test]
    async fn legacy_approval_required_does_not_enable_policy_when_rules_are_omitted() {
        let bus = InMemBus::default();
        let rs = RulesSnapshot {
            global: None,
            session: Vec::new(),
        };
        let mut c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "git", "args": ["push"]}),
        );
        c.approval_required = vec!["shell::exec".into()];

        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;

        assert_eq!(
            reply["block"], false,
            "omitted rules disable policy; legacy approval_required must not re-enable ask behavior",
        );
        assert!(bus.list_prefix("approvals", "sess_a/").await.is_empty());
    }

    #[tokio::test]
    async fn legacy_approval_required_cannot_bypass_a_matching_deny_rule() {
        let bus = InMemBus::default();
        let rs = policy(vec![r("shell::exec", "git push", Action::Deny)]);
        let mut c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "git", "args": ["push"]}),
        );
        c.approval_required = vec!["approval::resolve".into()];

        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;

        assert_eq!(reply["block"], true);
        assert_eq!(reply["status"], "denied");
        assert_eq!(reply["denial"]["kind"], "approval_rule_denied");
        assert!(bus.list_prefix("approvals", "sess_a/").await.is_empty());
    }

    #[tokio::test]
    async fn deny_rule_returns_block_true_structured_policy_denial() {
        let bus = InMemBus::default();
        let rs = policy(vec![r("shell::exec", "rm -rf*", Action::Deny)]);
        let c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "rm", "args": ["-rf", "/"]}),
        );
        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["block"], true);
        assert_eq!(reply["denial"]["kind"], "approval_rule_denied");
        assert_eq!(
            reply["denial"]["detail"]["rule"]["permission"],
            "shell::exec"
        );
        assert_eq!(reply["denial"]["detail"]["rule"]["pattern"], "rm -rf*");
        assert!(bus.list_prefix("approvals", "sess_a/").await.is_empty());
    }

    #[tokio::test]
    async fn no_match_defaults_to_ask_writes_pending() {
        let bus = InMemBus::default();
        let rs = policy(vec![]);
        let c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "git", "args": ["push"]}),
        );
        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["block"], true);
        assert_eq!(reply["status"], "pending");
        let stored = bus
            .get("approvals", "sess_a/tc-1")
            .await
            .expect("pending row");
        let r = Record::from_value(stored).unwrap();
        assert_eq!(r.status, Status::Pending);
    }

    #[tokio::test]
    async fn ask_rule_metadata_is_returned_and_persisted() {
        let bus = InMemBus::default();
        let rs = policy(vec![Rule {
            permission: "shell::exec".into(),
            pattern: "*".into(),
            action: Action::Ask,
            reason: Some("shell commands require review".into()),
        }]);
        let c = call(
            "tc-1",
            "shell::exec",
            json!({"command": "git", "args": ["push"]}),
        );
        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["status"], "pending");
        assert_eq!(reply["reason"], "shell commands require review");
        assert_eq!(reply["rule"]["permission"], "shell::exec");

        let stored = bus
            .get("approvals", "sess_a/tc-1")
            .await
            .expect("pending row");
        let record = Record::from_value(stored).unwrap();
        assert_eq!(
            record.reason.as_deref(),
            Some("shell commands require review")
        );
        assert_eq!(record.rule.as_ref().map(|r| r.pattern.as_str()), Some("*"));
    }

    #[tokio::test]
    async fn replay_on_pending_returns_in_flight_no_state_churn() {
        let bus = InMemBus::default();
        let rs = policy(vec![]);
        let c = call("tc-1", "shell::exec", json!({"command": "ls"}));

        handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        let r2 = handle_intercept(&bus, "approvals", &c, &rs, 2_000, 60_000).await;
        assert_eq!(r2["replay"], "in_flight");
        assert_eq!(r2["status"], "pending");

        let r = Record::from_value(bus.get("approvals", "sess_a/tc-1").await.unwrap()).unwrap();
        assert_eq!(
            r.expires_at, 61_000,
            "second call must NOT have re-written expires_at"
        );
    }

    #[tokio::test]
    async fn replay_on_in_flight_returns_in_flight_marker() {
        let bus = InMemBus::default();
        let in_flight = Record::pending(
            "tc-1".into(),
            "shell::exec".into(),
            json!({}),
            "sess_a".into(),
            0,
            60_000,
        )
        .in_flight(500);
        bus.set("approvals", "sess_a/tc-1", in_flight.to_value())
            .await
            .unwrap();

        let rs = policy(vec![]);
        let c = call("tc-1", "shell::exec", json!({}));
        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["replay"], "in_flight");
        assert_eq!(reply["status"], "in_flight");
    }

    #[tokio::test]
    async fn replay_on_done_returns_already_resolved_marker() {
        let bus = InMemBus::default();
        let done = Record::pending(
            "tc-1".into(),
            "shell::exec".into(),
            json!({}),
            "sess_a".into(),
            0,
            60_000,
        )
        .in_flight(500)
        .done(Outcome::Executed {
            result: json!({"ok": true}),
        });
        bus.set("approvals", "sess_a/tc-1", done.to_value())
            .await
            .unwrap();

        let rs = policy(vec![]);
        let c = call("tc-1", "shell::exec", json!({}));
        let reply = handle_intercept(&bus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["replay"], "already_resolved");
        assert_eq!(reply["status"], "done");
    }

    #[tokio::test]
    async fn state_write_failure_fails_closed_with_state_error_denial() {
        struct FailBus;
        #[async_trait::async_trait]
        impl StateBus for FailBus {
            async fn set(&self, _: &str, _: &str, _: Value) -> Result<(), iii_sdk::IIIError> {
                Err(iii_sdk::IIIError::Runtime("kv down".into()))
            }
            async fn get(&self, _: &str, _: &str) -> Option<Value> {
                None
            }
            async fn list_prefix(&self, _: &str, _: &str) -> Vec<Value> {
                Vec::new()
            }
            async fn delete(&self, _: &str, _: &str) -> Result<bool, iii_sdk::IIIError> {
                Ok(false)
            }
        }
        let rs = policy(vec![]);
        let c = call("tc-1", "shell::exec", json!({}));
        let reply = handle_intercept(&FailBus, "approvals", &c, &rs, 1_000, 60_000).await;
        assert_eq!(reply["block"], true);
        assert_eq!(reply["status"], "denied");
        assert_eq!(reply["denial"]["kind"], "state_error");
        assert_eq!(
            reply["denial"]["detail"]["phase"],
            "intercept_write_pending"
        );
    }
}
