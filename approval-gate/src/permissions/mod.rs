//! Inline permission rule evaluation — ported from harness policy engine.
//! Rules live in the `approval-gate` configuration entry (`GateDefaults::rules`).

mod compile;
mod default_rules;
mod types;

pub use compile::{
    compile_rule, match_constraints, match_function_id, CompiledRule, ConstraintMatch,
};
pub use types::{Action, ConstraintSpec, Decision, RuleSpec};

use compile::CompileError;
use serde_json::Value;
use tracing::warn;

use crate::types::PermissionMode;

pub fn rule_applies_in_mode(rule: &compile::CompiledRule, mode: PermissionMode) -> bool {
    match &rule.modes {
        None => true,
        Some(modes) => modes.contains(&mode),
    }
}

/// Globs from allow rules explicitly scoped to auto mode — seeds per-session
/// `always_allow` on first mutation.
pub fn auto_allow_seed_from_specs(specs: &[RuleSpec]) -> Vec<String> {
    specs
        .iter()
        .filter_map(|spec| match spec {
            RuleSpec::Structured {
                function,
                action: Action::Allow,
                modes: Some(modes),
                ..
            } if modes.contains(&PermissionMode::Auto) => Some(function.clone()),
            _ => None,
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Permissions {
    rules: Vec<CompiledRule>,
}

impl Permissions {
    pub fn empty() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    pub fn compile(specs: &[RuleSpec]) -> Result<Self, CompileError> {
        let mut rules = Vec::with_capacity(specs.len());
        let mut seen = std::collections::HashSet::new();
        for (idx, spec) in specs.iter().enumerate() {
            let rule = compile_rule(spec, idx)?;
            if !seen.insert(rule.rule_id.clone()) {
                warn!(rule_id = %rule.rule_id, "duplicate rule_id in approval-gate rules");
            }
            rules.push(rule);
        }
        Ok(Self { rules })
    }

    /// Tolerant compile: skip invalid entries; return empty on total failure.
    pub fn compile_tolerant(specs: &[RuleSpec]) -> Self {
        match Self::compile(specs) {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "failed to compile permission rules; using empty policy");
                Self::empty()
            }
        }
    }

    pub fn check(&self, function_id: &str, args: &Value, mode: PermissionMode) -> Decision {
        for rule in &self.rules {
            if !rule_applies_in_mode(rule, mode) {
                continue;
            }
            if !match_function_id(rule, function_id) {
                continue;
            }
            let (outcome, matched) = match_constraints(args, &rule.constraints);
            if matches!(outcome, ConstraintMatch::Mismatch) {
                continue;
            }
            return match rule.action {
                Action::Allow => Decision::Allow {
                    rule_id: rule.rule_id.clone(),
                },
                Action::Deny => Decision::Deny {
                    rule_id: rule.rule_id.clone(),
                    matched_constraint: matched,
                },
            };
        }
        Decision::NeedsApproval
    }
}

/// Parse one rule from a JSON config value.
pub fn parse_rule_spec(value: &Value) -> Option<RuleSpec> {
    if let Some(s) = value.as_str() {
        return Some(RuleSpec::Shorthand(s.to_string()));
    }
    let obj = value.as_object()?;
    let function = obj.get("function")?.as_str()?.to_string();
    let action = match obj.get("action")?.as_str()? {
        "allow" => Action::Allow,
        "deny" => Action::Deny,
        _ => return None,
    };
    let rule_id = obj
        .get("rule_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let modes = obj.get("modes").and_then(|v| {
        v.as_array().map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|s| match s {
                    "manual" => Some(PermissionMode::Manual),
                    "auto" => Some(PermissionMode::Auto),
                    "full" => Some(PermissionMode::Full),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
    });
    let args = obj
        .get("args")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(field, c)| parse_constraint(c).map(|spec| (field.clone(), spec)))
                .collect()
        })
        .unwrap_or_default();
    Some(RuleSpec::Structured {
        rule_id,
        function,
        action,
        modes,
        args,
    })
}

fn parse_constraint(value: &Value) -> Option<ConstraintSpec> {
    let obj = value.as_object()?;
    if let Some(v) = obj.get("equals") {
        return Some(ConstraintSpec::Equals(v.clone()));
    }
    if let Some(s) = obj.get("matches").and_then(Value::as_str) {
        return Some(ConstraintSpec::Matches(s.to_string()));
    }
    None
}

pub fn parse_rules_from_config(value: &Value) -> Vec<RuleSpec> {
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter().filter_map(parse_rule_spec).collect()
}

pub fn default_rule_specs() -> Vec<RuleSpec> {
    default_rules::default_rule_strings()
        .into_iter()
        .map(|s| RuleSpec::Shorthand(s.to_string()))
        .collect()
}

pub fn default_permissions() -> Permissions {
    Permissions::compile(&default_rule_specs()).expect("built-in rules compile")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kind(rules: &[&str], fid: &str, args: Value) -> Decision {
        let specs: Vec<RuleSpec> = rules
            .iter()
            .map(|s| RuleSpec::Shorthand((*s).to_string()))
            .collect();
        Permissions::compile(&specs)
            .unwrap()
            .check(fid, &args, PermissionMode::Manual)
    }

    #[test]
    fn empty_permissions_gates_every_call() {
        assert!(matches!(
            Permissions::empty().check("shell::exec", &json!({}), PermissionMode::Manual),
            Decision::NeedsApproval
        ));
    }

    #[test]
    fn bare_string_allow() {
        let d = kind(&["shell::fs::ls"], "shell::fs::ls", json!({}));
        assert_eq!(
            d,
            Decision::Allow {
                rule_id: "shell::fs::ls".into()
            }
        );
    }

    #[test]
    fn deny_shorthand() {
        assert!(matches!(
            kind(&["!state::set"], "state::set", json!({})),
            Decision::Deny { .. }
        ));
    }

    #[test]
    fn catch_all_star_allows() {
        let d = kind(&["*"], "shell::exec", json!({ "command": "rm -rf /" }));
        assert_eq!(
            d,
            Decision::Allow {
                rule_id: "*".into()
            }
        );
    }

    #[test]
    fn catch_all_after_deny_still_denies() {
        assert!(matches!(
            kind(&["!state::set", "*"], "state::set", json!({})),
            Decision::Deny { .. }
        ));
        assert!(matches!(
            kind(&["!state::set", "*"], "shell::exec", json!({})),
            Decision::Allow { .. }
        ));
    }

    #[test]
    fn glob_provider_needs_approval_by_default() {
        assert!(matches!(
            default_permissions().check(
                "provider::anthropic::chat",
                &json!({}),
                PermissionMode::Manual
            ),
            Decision::NeedsApproval
        ));
    }

    #[test]
    fn web_fetch_needs_approval_by_default() {
        assert!(matches!(
            default_permissions().check("web::fetch", &json!({}), PermissionMode::Manual),
            Decision::NeedsApproval
        ));
    }

    #[test]
    fn unknown_function_needs_approval() {
        assert!(matches!(
            default_permissions().check("shell::run", &json!({}), PermissionMode::Manual),
            Decision::NeedsApproval
        ));
    }

    #[test]
    fn structured_equals_constraint() {
        let specs = vec![RuleSpec::Structured {
            rule_id: None,
            function: "f".into(),
            action: Action::Allow,
            modes: None,
            args: vec![("n".into(), ConstraintSpec::Equals(json!(0)))],
        }];
        let p = Permissions::compile(&specs).unwrap();
        assert!(matches!(
            p.check("f", &json!({ "n": 0 }), PermissionMode::Manual),
            Decision::Allow { .. }
        ));
        assert!(matches!(
            p.check("f", &json!({ "n": "0" }), PermissionMode::Manual),
            Decision::NeedsApproval
        ));
    }
}
