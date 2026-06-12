//! Denial envelope assembly + text rendering (the `reason` strings are
//! ported verbatim from harness/src/approval-gate/denial.ts — they are
//! written for the model as much as the human).

use serde_json::Value;

use crate::redact::redact;
use crate::types::{
    text_block, DenialEnvelope, DeniedBy, MatchedConstraint, TextBlock, DENIAL_SCHEMA_VERSION,
};

pub const HUMAN_ONLY_RULE_ID: &str = "human_only_function";

pub fn reason_for_permissions_deny(
    function_id: &str,
    rule_id: &str,
    matched: Option<&MatchedConstraint>,
) -> String {
    match matched {
        Some(m) => format!(
            "Permission denied: {function_id} matched rule {rule_id} on {} {} {}. Try different arguments or use a different function.",
            m.field,
            m.operator,
            serde_json::to_string(&m.value).unwrap_or_else(|_| "null".to_string())
        ),
        None => format!(
            "Permission denied: {function_id} matched rule {rule_id}. This function is blocked by policy; try a different function."
        ),
    }
}

pub fn permissions_deny_envelope(
    function_id: &str,
    rule_id: &str,
    matched_constraint: Option<MatchedConstraint>,
    args: &Value,
) -> DenialEnvelope {
    DenialEnvelope {
        schema_version: DENIAL_SCHEMA_VERSION,
        status: "denied".to_string(),
        denied_by: DeniedBy::Permissions,
        function_id: function_id.to_string(),
        rule_id: Some(rule_id.to_string()),
        rule_action: Some("deny".to_string()),
        reason: reason_for_permissions_deny(function_id, rule_id, matched_constraint.as_ref()),
        matched_constraint,
        args_excerpt: Some(redact(args)),
    }
}

/// Operator denial via `approval::resolve`. The excerpt comes from the
/// pending record, which is already redacted — it is passed through, not
/// re-redacted.
pub fn user_deny_envelope(
    function_id: &str,
    reason: Option<&str>,
    args_excerpt: Option<Value>,
) -> DenialEnvelope {
    DenialEnvelope {
        schema_version: DENIAL_SCHEMA_VERSION,
        status: "denied".to_string(),
        denied_by: DeniedBy::User,
        function_id: function_id.to_string(),
        rule_id: None,
        rule_action: None,
        matched_constraint: None,
        args_excerpt,
        reason: reason
            .filter(|r| !r.is_empty())
            .unwrap_or("Rejected by operator.")
            .to_string(),
    }
}

/// Fail-closed transport failure: a crashed policy worker or state outage
/// must deny, never wave calls through or hold blind.
pub fn gate_unavailable_envelope(function_id: &str, reason: &str) -> DenialEnvelope {
    DenialEnvelope {
        schema_version: DENIAL_SCHEMA_VERSION,
        status: "denied".to_string(),
        denied_by: DeniedBy::GateUnavailable,
        function_id: function_id.to_string(),
        rule_id: None,
        rule_action: None,
        matched_constraint: None,
        args_excerpt: None,
        reason: reason.to_string(),
    }
}

/// Self-escalation defense: `approval::*` / `configuration::*` targets are
/// operator surfaces, denied even under `mode: "full"`.
pub fn human_only_denial(function_id: &str, args: &Value) -> DenialEnvelope {
    permissions_deny_envelope(function_id, HUMAN_ONLY_RULE_ID, None, args)
}

/// One text block carrying the envelope's reason — the `content` of a
/// deny/timeout function_result (the envelope itself rides in `details`).
pub fn render_text(envelope: &DenialEnvelope) -> Vec<TextBlock> {
    vec![text_block(envelope.reason.clone())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permissions_envelope_carries_rule_and_redacted_args() {
        let envelope = permissions_deny_envelope(
            "shell::run",
            "no_shell",
            Some(MatchedConstraint {
                field: "cmd".into(),
                operator: "contains".into(),
                value: json!("rm"),
            }),
            &json!({ "cmd": "rm -rf /", "api_key": "sk_live" }),
        );
        assert_eq!(envelope.denied_by, DeniedBy::Permissions);
        assert_eq!(envelope.rule_id.as_deref(), Some("no_shell"));
        assert_eq!(envelope.rule_action.as_deref(), Some("deny"));
        assert_eq!(
            envelope.args_excerpt.unwrap()["api_key"],
            json!("<redacted>")
        );
        assert!(envelope
            .reason
            .contains("matched rule no_shell on cmd contains \"rm\""));
        assert!(envelope.reason.contains("Try different arguments"));
    }

    #[test]
    fn permissions_reason_without_constraint() {
        let reason = reason_for_permissions_deny("shell::run", "no_shell", None);
        assert!(reason.contains("blocked by policy"));
    }

    #[test]
    fn user_envelope_defaults_reason_and_passes_excerpt_through() {
        let envelope = user_deny_envelope("shell::run", None, Some(json!({ "cmd": "ls" })));
        assert_eq!(envelope.denied_by, DeniedBy::User);
        assert_eq!(envelope.reason, "Rejected by operator.");
        assert_eq!(envelope.args_excerpt, Some(json!({ "cmd": "ls" })));

        let custom = user_deny_envelope("shell::run", Some("too risky"), None);
        assert_eq!(custom.reason, "too risky");
    }

    #[test]
    fn human_only_uses_the_reserved_rule_id() {
        let envelope = human_only_denial("approval::set_mode", &json!({}));
        assert_eq!(envelope.rule_id.as_deref(), Some(HUMAN_ONLY_RULE_ID));
        assert_eq!(envelope.denied_by, DeniedBy::Permissions);
    }

    #[test]
    fn render_text_is_one_text_block_with_the_reason() {
        let envelope = gate_unavailable_envelope("shell::run", "policy unreachable: boom");
        let blocks = render_text(&envelope);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, "text");
        assert_eq!(blocks[0].text, "policy unreachable: boom");
    }
}
