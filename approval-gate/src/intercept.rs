//! Intercept decision flow.
//!
//! Pure decision helpers + the async [`handle_intercept`] that writes
//! the pending record. Together they answer the question every hook
//! event triggers: "what should the gate do with this function call?"
//!
//! Three layers run, in order:
//!
//! 1. **Policy rules** ([`apply_policy_rules`]) — operator-configured
//!    layered ruleset. `Allow` and `Deny` short-circuit; `Ask` (and
//!    no-match) falls through.
//! 2. **Interceptor rule** ([`decide_intercept_action`]) — per-function
//!    config. Decides between `Pass`, `Pause` (no classifier), and
//!    `Classify { classifier_fn, … }`.
//! 3. **Classifier reply** ([`interpret_classifier_reply`]) — parses the
//!    classifier function's JSON response and maps it back to either an
//!    immediate `Auto` (pass), an immediate `Deny`, or `Ask` (fall back
//!    to user prompt via `handle_intercept`).
//!
//! This module owns only the decision types and `handle_intercept`. The
//! wiring (closure body in `register`) lives in `register.rs`.

use serde_json::{json, Value};

use crate::config::InterceptorRule;
use crate::lifecycle::{build_pending_record, is_terminal_status};
use crate::rules;
use crate::state::StateBus;
use crate::wire::{pending_key, Denial, IncomingCall};

/// What the subscriber should do with an incoming call. Decided by the
/// matching interceptor rule (authoritative) with a fallback to the run's
/// `approval_required` list when no rule exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InterceptAction {
    /// No rule, no `approval_required` listing — let the call through.
    Pass,
    /// Pause and create a pending record; no classifier consulted.
    Pause,
    /// Run the classifier first; on `ask`, pause; on `auto`, pass; on `deny`, block.
    Classify {
        classifier_fn: String,
        classifier_timeout_ms: u64,
    },
}

/// Pure decision: given a matching rule (or none) and whether the run
/// explicitly listed this function id in `approval_required`, what should
/// the subscriber do? Interceptor rules are authoritative — an operator
/// who registered a rule meant for every call to go through it, regardless
/// of per-run opt-in.
pub(crate) fn decide_intercept_action(
    rule: Option<&InterceptorRule>,
    requires_approval: bool,
) -> InterceptAction {
    match rule {
        Some(r) if r.classifier.as_ref().is_some_and(|s| !s.is_empty()) => {
            InterceptAction::Classify {
                classifier_fn: r.classifier.clone().unwrap(),
                classifier_timeout_ms: r.classifier_timeout_ms,
            }
        }
        Some(_) => InterceptAction::Pause,
        None if requires_approval => InterceptAction::Pause,
        None => InterceptAction::Pass,
    }
}

/// Outcome of the policy-rules pre-check that runs before the per-function
/// [`InterceptorRule`] flow. `Allow` and `Deny` short-circuit the
/// subscriber with a final reply; `FallThrough` defers to the existing
/// interceptor logic (classifier or pause).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PolicyOutcome {
    Allow,
    Deny {
        rule_permission: String,
        rule_pattern: String,
    },
    FallThrough,
}

/// Apply the layered policy rules to an incoming function id. Pure
/// function — no I/O, no clock. Extracted from the subscriber closure
/// so the decision branch can be unit-tested independently.
pub(crate) fn apply_policy_rules(
    rules: &rules::Ruleset,
    function_id: &str,
) -> PolicyOutcome {
    match rules::evaluate(function_id, "*", rules) {
        Some(rule) => match rule.action {
            rules::Action::Allow => PolicyOutcome::Allow,
            rules::Action::Deny => PolicyOutcome::Deny {
                rule_permission: rule.permission.clone(),
                rule_pattern: rule.pattern.clone(),
            },
            rules::Action::Ask => PolicyOutcome::FallThrough,
        },
        None => PolicyOutcome::FallThrough,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClassifierDecision {
    Auto,
    Deny(Denial),
    Ask,
}

/// Parse classifier JSON (`decision` tag: auto | deny | ask). On `deny`
/// the reply may carry `reason` (free-form classifier text); both that
/// and the calling `classifier_fn` get folded into a [`Denial::Policy`].
pub(crate) fn interpret_classifier_reply(
    value: &Value,
    classifier_fn: &str,
) -> Result<ClassifierDecision, ()> {
    let tag = value.get("decision").and_then(Value::as_str).ok_or(())?;
    match tag {
        "auto" => Ok(ClassifierDecision::Auto),
        "deny" => {
            let classifier_reason = value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("denied")
                .to_string();
            // Transient mapping: classifier reason/fn stored in the renamed
            // Denial::Policy fields. The whole classifier surface is deleted
            // in T5; for now this just keeps the build green.
            Ok(ClassifierDecision::Deny(Denial::Policy {
                rule_permission: classifier_fn.to_string(),
                rule_pattern: classifier_reason,
            }))
        }
        "ask" => Ok(ClassifierDecision::Ask),
        _ => Err(()),
    }
}

/// Decide whether a call is gated; if so, write a pending record and return
/// the structured pending hook reply. If not gated, return `{block: false}`
/// and do nothing.
///
/// Stamps `session_id` onto the persisted record so the timeout sweeper can
/// emit `approval_resolved` to the right session stream without consulting
/// the storage layer's keys.
///
/// State-write failure is treated as fail-closed: the gate replies
/// `{block:true, status:"denied"}` so a transient kv outage cannot silently
/// bypass an approval check.
pub async fn handle_intercept(
    bus: &dyn StateBus,
    state_scope: &str,
    call: &IncomingCall,
    now_ms: u64,
    timeout_ms: u64,
    force_pending: bool,
) -> Value {
    if !force_pending && !call.requires_approval() {
        return json!({ "block": false });
    }

    // Defense in depth: if a record for this (session, call_id) already
    // exists, don't blow it away. Re-intercept of an already-decided call
    // would otherwise revert a terminal record back to `pending`, losing
    // the audit trail and any `delivered_in_turn_id` stamp.
    let key = pending_key(&call.session_id, &call.function_call_id);
    if let Some(existing) = bus.get(state_scope, &key).await {
        let status = existing
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if is_terminal_status(&status) {
            // Replay of an already-resolved call: the prior status carries
            // the meaning. No fresh Denial is synthesized — consumers that
            // need to render the historical decision read the persisted
            // record via approval::lookup_record.
            return json!({
                "block": true,
                "status": status,
                "replay": "already_resolved",
                "call_id": call.function_call_id,
                "function_id": call.function_id,
            });
        }
        if status == "pending" || status == "approved" {
            // Replay of an in-flight intercept — keep the existing row,
            // re-emit the pending reply. No state churn.
            return json!({
                "block": true,
                "status": "pending",
                "replay": "in_flight",
                "call_id": call.function_call_id,
                "function_id": call.function_id,
            });
        }
    }

    let mut record = build_pending_record(
        &call.function_call_id,
        &call.function_id,
        &call.args,
        now_ms,
        timeout_ms,
    );
    if let Some(obj) = record.as_object_mut() {
        obj.insert("session_id".into(), Value::String(call.session_id.clone()));
    }
    if let Err(err) = bus
        .set(
            state_scope,
            &pending_key(&call.session_id, &call.function_call_id),
            record,
        )
        .await
    {
        tracing::error!(
            "approval-gate: failed to write pending record for {}/{}: {err} — failing closed",
            call.session_id,
            call.function_call_id
        );
        let denial = Denial::StateError {
            phase: "intercept_write_pending".to_string(),
            error: err.to_string(),
        };
        return json!({
            "block": true,
            "denial": denial,
            "status": "denied",
            "call_id": call.function_call_id,
            "function_id": call.function_id,
        });
    }
    json!({
        "block": true,
        "status": "pending",
        "call_id": call.function_call_id,
        "function_id": call.function_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interpret_classifier_reply_reads_decision_tags() {
        assert!(matches!(
            interpret_classifier_reply(&json!({"decision": "auto"}), "shell::classify_argv"),
            Ok(ClassifierDecision::Auto)
        ));
        match interpret_classifier_reply(
            &json!({"decision":"deny","reason":"nope"}),
            "shell::classify_argv",
        ) {
            Ok(ClassifierDecision::Deny(Denial::Policy {
                rule_permission,
                rule_pattern,
            })) => {
                // Per the transient mapping in interpret_classifier_reply.
                assert_eq!(rule_pattern, "nope");
                assert_eq!(rule_permission, "shell::classify_argv");
            }
            o => panic!("expected Policy denial {:?}", o),
        }
        assert!(matches!(
            interpret_classifier_reply(
                &json!({"decision":"ask","summary":"x"}),
                "shell::classify_argv"
            ),
            Ok(ClassifierDecision::Ask)
        ));
        assert!(interpret_classifier_reply(&json!({}), "shell::classify_argv").is_err());
    }

    /// An operator-registered rule is authoritative: every call to that
    /// function id runs through the classifier, even when the run's
    /// `approval_required` list is empty.
    #[test]
    fn decide_intercept_action_classifies_when_rule_has_classifier_regardless_of_approval_required(
    ) {
        let rule = InterceptorRule {
            function_id: "shell::exec".into(),
            classifier: Some("shell::classify_argv".into()),
            classifier_timeout_ms: 2000,
            inject_approval_marker: true,
            marker_target_verified: true,
        };
        let action = decide_intercept_action(Some(&rule), false);
        assert_eq!(
            action,
            InterceptAction::Classify {
                classifier_fn: "shell::classify_argv".into(),
                classifier_timeout_ms: 2000,
            }
        );
        assert_eq!(action, decide_intercept_action(Some(&rule), true));
    }

    #[test]
    fn decide_intercept_action_pauses_when_rule_has_no_classifier_regardless_of_approval_required()
    {
        let rule = InterceptorRule {
            function_id: "shell::fs::write".into(),
            classifier: None,
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
            marker_target_verified: false,
        };
        assert_eq!(
            decide_intercept_action(Some(&rule), false),
            InterceptAction::Pause
        );
        assert_eq!(
            decide_intercept_action(Some(&rule), true),
            InterceptAction::Pause
        );
    }

    #[test]
    fn decide_intercept_action_pauses_when_no_rule_but_run_listed_approval_required() {
        assert_eq!(decide_intercept_action(None, true), InterceptAction::Pause);
    }

    #[test]
    fn decide_intercept_action_passes_when_no_rule_and_not_approval_required() {
        assert_eq!(decide_intercept_action(None, false), InterceptAction::Pass);
    }

    #[test]
    fn decide_intercept_action_classifier_empty_string_treated_as_no_classifier() {
        let rule = InterceptorRule {
            function_id: "shell::exec".into(),
            classifier: Some(String::new()),
            classifier_timeout_ms: 2000,
            inject_approval_marker: false,
            marker_target_verified: false,
        };
        assert_eq!(
            decide_intercept_action(Some(&rule), false),
            InterceptAction::Pause
        );
    }

    #[test]
    fn apply_policy_rules_empty_ruleset_falls_through() {
        let rs: rules::Ruleset = vec![];
        assert_eq!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::FallThrough
        );
    }

    #[test]
    fn apply_policy_rules_allow_short_circuits() {
        let rs: rules::Ruleset = vec![rules::Rule {
            permission: "shell::exec".into(),
            pattern: "*".into(),
            action: rules::Action::Allow,
        }];
        assert_eq!(apply_policy_rules(&rs, "shell::exec"), PolicyOutcome::Allow);
    }

    #[test]
    fn apply_policy_rules_deny_carries_matched_rule_identity() {
        let rs: rules::Ruleset = vec![rules::Rule {
            permission: "shell::*".into(),
            pattern: "*".into(),
            action: rules::Action::Deny,
        }];
        assert_eq!(
            apply_policy_rules(&rs, "shell::fs::write"),
            PolicyOutcome::Deny {
                rule_permission: "shell::*".into(),
                rule_pattern: "*".into(),
            }
        );
    }

    #[test]
    fn apply_policy_rules_ask_falls_through_to_interceptor_flow() {
        // Ask means "no decision from this layer — let the next handle it".
        let rs: rules::Ruleset = vec![rules::Rule {
            permission: "shell::exec".into(),
            pattern: "*".into(),
            action: rules::Action::Ask,
        }];
        assert_eq!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::FallThrough
        );
    }

    #[test]
    fn apply_policy_rules_last_matching_wins() {
        // Later-listed more-specific rule overrides earlier permissive default.
        let rs: rules::Ruleset = vec![
            rules::Rule {
                permission: "*".into(),
                pattern: "*".into(),
                action: rules::Action::Allow,
            },
            rules::Rule {
                permission: "shell::exec".into(),
                pattern: "*".into(),
                action: rules::Action::Deny,
            },
        ];
        assert!(matches!(
            apply_policy_rules(&rs, "shell::exec"),
            PolicyOutcome::Deny { .. }
        ));
        assert_eq!(
            apply_policy_rules(&rs, "approval::resolve"),
            PolicyOutcome::Allow
        );
    }
}
