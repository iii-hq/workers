//! Approval gate. Subscribes to `agent::before_function_call` and decides
//! every call via the layered rules engine. Allow → `{block:false}`.
//! Deny → `{block:true, denial:ApprovalRuleDenied{rule}}`. Ask → write a Pending record and wait for
//! `approval::resolve`.

pub mod config;
pub mod delivery;
pub mod intercept;
pub mod manifest;
pub mod record;
pub mod register;
pub mod resolve;
pub mod rules;
pub mod state;
pub mod wire;

pub use config::WorkerConfig;
pub use delivery::{
    handle_consume, handle_list_pending, handle_sweep_session, handle_tick_timeouts,
    CONSUME_DEFAULT_LIMIT,
};
pub use intercept::handle_intercept;
pub use record::{Outcome, Record, Status, IN_FLIGHT_GRACE_MS};
pub use register::{
    build_approval_resolved_event, register, run_tick, IIIWaker, OrchestratorWaker, Refs,
    FN_CONSUME, FN_LIST_PENDING, FN_LOOKUP_RECORD, FN_RESOLVE, FN_SWEEP_SESSION, FN_TICK_TIMEOUTS,
    STATE_SCOPE,
};
pub use resolve::{handle_lookup_record, handle_resolve};
pub use rules::LayeredRules;
pub use state::{FunctionExecutor, IiiFunctionExecutor, IiiStateBus, StateBus};
pub use wire::{
    block_reply_for, extract_call, parse_call, pending_key, state_error_reply, Decision, Denial,
    ExtractCallError, IncomingCall, WireDecision,
};

/// Subscriber's terminal verdict for an incoming call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Allow {
        rule: Option<rules::RuleMatch>,
    },
    Deny {
        denial: Denial,
        rule: Option<rules::RuleMatch>,
    },
    Ask {
        rule: Option<rules::RuleMatch>,
    },
}

/// Apply the layered rules to an incoming call. Omitted policy allows all;
/// explicit policy is first-match-wins with no-match defaulting to Ask.
/// Session `allow + always` overlays may override Ask/no-match, but never
/// any matching global deny.
pub(crate) fn verdict_for(
    function_id: &str,
    args: &serde_json::Value,
    rules: &rules::RulesSnapshot,
) -> Verdict {
    if function_id.starts_with("approval::") {
        return Verdict::Allow { rule: None };
    }
    let Some(global) = &rules.global else {
        return Verdict::Allow { rule: None };
    };
    let ctx = rules::match_context(function_id, args);
    let global_deny = rules::any_matching_deny(global, rules::RuleSource::Global, &ctx);
    let global_match = rules::evaluate_context(global, rules::RuleSource::Global, &ctx);
    if let Some(rule_match) = global_match {
        return match rule_match.action {
            rules::Action::Allow => Verdict::Allow {
                rule: Some(rule_match),
            },
            rules::Action::Deny => Verdict::Deny {
                denial: Denial::ApprovalRuleDenied {
                    rule: rule_match.clone(),
                },
                rule: Some(rule_match),
            },
            rules::Action::Ask => {
                guarded_session_or_ask(&ctx, &rules.session, global_deny, Some(rule_match))
            }
        };
    }
    guarded_session_or_ask(&ctx, &rules.session, global_deny, None)
}

fn guarded_session_or_ask(
    ctx: &rules::MatchContext,
    session_rules: &rules::Ruleset,
    global_deny: Option<rules::RuleMatch>,
    ask_rule: Option<rules::RuleMatch>,
) -> Verdict {
    if global_deny.is_none() {
        if let Some(session_match) =
            rules::evaluate_context(session_rules, rules::RuleSource::Session, ctx)
        {
            if session_match.action == rules::Action::Allow {
                return Verdict::Allow {
                    rule: Some(session_match),
                };
            }
        }
    }
    Verdict::Ask { rule: ask_rule }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(permission: &str, pattern: &str, action: rules::Action) -> rules::Rule {
        rules::Rule {
            permission: permission.to_string(),
            pattern: pattern.to_string(),
            action,
            reason: None,
        }
    }

    #[test]
    fn omitted_rules_allow_all_non_approval_calls() {
        let snapshot = rules::RulesSnapshot {
            global: None,
            session: vec![],
        };
        assert!(matches!(
            verdict_for("shell::exec", &json!({"command": "rm -rf /"}), &snapshot),
            Verdict::Allow { rule: None }
        ));
    }

    #[test]
    fn explicit_empty_rules_ask_on_no_match() {
        let snapshot = rules::RulesSnapshot {
            global: Some(vec![]),
            session: vec![],
        };
        assert!(matches!(
            verdict_for("shell::exec", &json!({"command": "git status"}), &snapshot),
            Verdict::Ask { rule: None }
        ));
    }

    #[test]
    fn approval_functions_self_allow_before_rules() {
        let snapshot = rules::RulesSnapshot {
            global: Some(vec![r("approval::*", "*", rules::Action::Ask)]),
            session: vec![],
        };
        assert!(matches!(
            verdict_for("approval::resolve", &json!({}), &snapshot),
            Verdict::Allow { rule: None }
        ));
    }

    #[test]
    fn session_allow_overrides_ask_but_not_matching_global_deny() {
        let args = json!({"command": "git", "args": ["push"]});
        let ask_snapshot = rules::RulesSnapshot {
            global: Some(vec![r("shell::exec", "*", rules::Action::Ask)]),
            session: vec![r("shell::exec", "git push", rules::Action::Allow)],
        };
        assert!(matches!(
            verdict_for("shell::exec", &args, &ask_snapshot),
            Verdict::Allow {
                rule: Some(rules::RuleMatch {
                    source: rules::RuleSource::Session,
                    ..
                })
            }
        ));

        let deny_snapshot = rules::RulesSnapshot {
            global: Some(vec![
                r("shell::exec", "*", rules::Action::Ask),
                r("shell::exec", "git push", rules::Action::Deny),
            ]),
            session: vec![r("shell::exec", "git push", rules::Action::Allow)],
        };
        assert!(matches!(
            verdict_for("shell::exec", &args, &deny_snapshot),
            Verdict::Ask { .. }
        ));
    }

    #[test]
    fn first_matching_global_deny_hard_blocks_even_when_session_allow_matches() {
        let snapshot = rules::RulesSnapshot {
            global: Some(vec![r("shell::exec", "git push", rules::Action::Deny)]),
            session: vec![r("shell::exec", "git push", rules::Action::Allow)],
        };

        assert!(matches!(
            verdict_for(
                "shell::exec",
                &json!({"command": "git", "args": ["push"]}),
                &snapshot,
            ),
            Verdict::Deny {
                rule: Some(rules::RuleMatch {
                    source: rules::RuleSource::Global,
                    action: rules::Action::Deny,
                    ..
                }),
                ..
            }
        ));
    }
}
