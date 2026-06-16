//! `policy::check_permissions` client — the gate's yaml-rule fallback.
//! Reply mapping (approval-gate.md § Yaml policy dependency):
//! `allow`/`deny`/`needs_approval` map directly; **unparseable replies
//! degrade to needs_approval** (a human look is the safe reading of
//! "don't know") while **transport failures and timeouts fail closed**
//! to deny (`gate_unavailable`).

use std::time::Duration;

use iii_sdk::{TriggerRequest, III};
use serde_json::{json, Value};

use crate::types::MatchedConstraint;

#[derive(Debug, Clone, PartialEq)]
pub enum PolicyOutcome {
    Allow,
    Deny {
        rule_id: String,
        matched_constraint: Option<MatchedConstraint>,
    },
    NeedsApproval,
    Unavailable(String),
}

pub async fn check(iii: &III, function_id: &str, args: &Value, timeout_ms: u64) -> PolicyOutcome {
    let payload = json!({ "function_id": function_id, "args": args });
    // Belt and braces: the trigger timeout should bound the call, but a
    // misbehaving transport must not stretch the hook past its budget.
    let reply = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        iii.trigger(TriggerRequest {
            function_id: "policy::check_permissions".into(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        }),
    )
    .await;
    match reply {
        Err(_elapsed) => PolicyOutcome::Unavailable("policy consult timed out".to_string()),
        Ok(Err(e)) => PolicyOutcome::Unavailable(format!("policy unreachable: {e}")),
        Ok(Ok(value)) => parse_reply(&value),
    }
}

pub fn parse_reply(value: &Value) -> PolicyOutcome {
    match value.get("decision").and_then(Value::as_str) {
        Some("allow") => PolicyOutcome::Allow,
        Some("deny") => PolicyOutcome::Deny {
            rule_id: value
                .get("rule_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            matched_constraint: value
                .get("matched_constraint")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
        },
        Some("needs_approval") => PolicyOutcome::NeedsApproval,
        _ => PolicyOutcome::NeedsApproval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_allow_deny_needs_approval() {
        assert_eq!(
            parse_reply(&json!({ "decision": "allow", "rule_id": "r1" })),
            PolicyOutcome::Allow
        );
        assert_eq!(
            parse_reply(&json!({
                "decision": "deny",
                "rule_id": "r2",
                "matched_constraint": { "field": "cmd", "operator": "eq", "value": "rm" }
            })),
            PolicyOutcome::Deny {
                rule_id: "r2".into(),
                matched_constraint: Some(MatchedConstraint {
                    field: "cmd".into(),
                    operator: "eq".into(),
                    value: json!("rm"),
                }),
            }
        );
        assert_eq!(
            parse_reply(&json!({ "decision": "needs_approval" })),
            PolicyOutcome::NeedsApproval
        );
    }

    #[test]
    fn unparseable_replies_degrade_to_needs_approval() {
        assert_eq!(parse_reply(&json!("garbage")), PolicyOutcome::NeedsApproval);
        assert_eq!(parse_reply(&Value::Null), PolicyOutcome::NeedsApproval);
        assert_eq!(
            parse_reply(&json!({ "decision": "maybe?" })),
            PolicyOutcome::NeedsApproval
        );
    }

    #[test]
    fn deny_tolerates_missing_rule_id_and_bad_constraint() {
        assert_eq!(
            parse_reply(&json!({ "decision": "deny", "matched_constraint": "not-an-object" })),
            PolicyOutcome::Deny {
                rule_id: String::new(),
                matched_constraint: None,
            }
        );
    }
}
