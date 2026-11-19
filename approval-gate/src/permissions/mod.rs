//! Inline permission rule evaluation — ported from harness policy engine.
//! Rules live in the `approval-gate` configuration entry (`GateDefaults::rules`).

mod compile;
mod default_rules;
mod types;

pub use compile::{
    CompiledRule, ConstraintMatch, compile_rule, match_constraints, match_function_id,
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

/// Splits a rule pattern into its engine namespace and id parts:
/// `laptop/shell::*` → `(Some("laptop"), "shell::*")`. Only a non-empty prefix
/// without `:` counts, so an id that holds a `/` (`openai-codex::codex/gpt`)
/// stays whole.
pub fn split_namespace_pattern(pattern: &str) -> (Option<&str>, &str) {
    match pattern.split_once('/') {
        Some((namespace, id)) if !namespace.is_empty() && !namespace.contains(':') => {
            (Some(namespace), id)
        }
        _ => (None, pattern),
    }
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
        self.check_in(None, function_id, args, mode)
    }

    /// Like [`Self::check`] for a call in `namespace`. A bare rule matches
    /// every namespace; a `<namespace>/` rule matches only its own.
    pub fn check_in(
        &self,
        namespace: Option<&str>,
        function_id: &str,
        args: &Value,
        mode: PermissionMode,
    ) -> Decision {
        for rule in &self.rules {
            if !rule_applies_in_mode(rule, mode) {
                continue;
            }
            let in_namespace = match (rule.namespace.as_deref(), namespace) {
                (None, _) => true,
                (Some(rule_ns), Some(ns)) => rule_ns == ns,
                // Fails closed for a call without namespace: a qualified deny matches, a qualified allow does not.
                (Some(_), None) => rule.action == Action::Deny,
            };
            if !in_namespace || !match_function_id(rule, function_id) {
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

    #[test]
    fn an_empty_bare_allow_compiles_as_before_and_an_empty_qualified_id_is_an_error() {
        assert!(compile_rule(&RuleSpec::Shorthand(String::new()), 0).is_ok());
        for raw in ["laptop/", "!laptop/", "!"] {
            assert!(
                compile_rule(&RuleSpec::Shorthand(raw.into()), 0).is_err(),
                "{raw}"
            );
        }
        let structured = |function: &str| RuleSpec::Structured {
            rule_id: None,
            function: function.into(),
            action: Action::Allow,
            modes: None,
            args: Default::default(),
        };
        assert!(compile_rule(&structured("laptop/"), 0).is_err());
    }

    #[test]
    fn a_glob_in_the_namespace_qualifier_is_an_error() {
        let structured = RuleSpec::Structured {
            rule_id: None,
            function: "lap*/shell::*".into(),
            action: Action::Allow,
            modes: None,
            args: Default::default(),
        };
        for spec in [
            RuleSpec::Shorthand("!*/shell::exec".into()),
            RuleSpec::Shorthand("*/state::get".into()),
            structured,
        ] {
            let err = compile_rule(&spec, 0).unwrap_err().to_string();
            assert!(
                err.contains("namespace qualifier must be a plain namespace"),
                "{err}"
            );
        }
    }

    #[test]
    fn an_empty_namespace_qualifier_is_an_error() {
        let structured = RuleSpec::Structured {
            rule_id: None,
            function: "/shell::exec".into(),
            action: Action::Deny,
            modes: None,
            args: Default::default(),
        };
        for spec in [
            RuleSpec::Shorthand("!/shell::exec".into()),
            RuleSpec::Shorthand("/shell::exec".into()),
            structured,
        ] {
            let err = compile_rule(&spec, 0).unwrap_err().to_string();
            assert!(
                err.contains("namespace qualifier must be a plain namespace"),
                "{err}"
            );
        }
        assert!(compile_rule(&RuleSpec::Shorthand(String::new()), 0).is_ok());
        assert!(compile_rule(&RuleSpec::Shorthand("openai-codex::codex/gpt".into()), 0).is_ok());
    }

    #[test]
    fn a_doubled_slash_after_a_namespace_qualifier_is_an_error() {
        let structured = RuleSpec::Structured {
            rule_id: None,
            function: "laptop//shell::exec".into(),
            action: Action::Deny,
            modes: None,
            args: Default::default(),
        };
        for spec in [
            RuleSpec::Shorthand("!laptop//shell::exec".into()),
            RuleSpec::Shorthand("laptop//shell::exec".into()),
            structured,
        ] {
            let err = compile_rule(&spec, 0).unwrap_err().to_string();
            assert!(
                err.contains("namespace qualifier must be a plain namespace"),
                "{err}"
            );
        }
        assert!(
            compile_rule(
                &RuleSpec::Shorthand("laptop/openai-codex::codex/gpt".into()),
                0
            )
            .is_ok()
        );
    }

    #[test]
    fn split_namespace_pattern_takes_a_plain_prefix_before_the_first_slash() {
        assert_eq!(
            split_namespace_pattern("laptop/shell::*"),
            (Some("laptop"), "shell::*")
        );
        assert_eq!(split_namespace_pattern("shell::*"), (None, "shell::*"));
        assert_eq!(
            split_namespace_pattern("openai-codex::codex/gpt"),
            (None, "openai-codex::codex/gpt")
        );
        assert_eq!(
            split_namespace_pattern("/shell::exec"),
            (None, "/shell::exec")
        );
    }

    #[test]
    fn bare_rule_matches_every_namespace_and_qualified_rule_only_its_own() {
        let p = Permissions::compile(&[
            RuleSpec::Shorthand("laptop/shell::*".into()),
            RuleSpec::Shorthand("state::get".into()),
        ])
        .unwrap();
        let m = PermissionMode::Manual;
        for ns in [None, Some("default"), Some("laptop")] {
            assert!(matches!(
                p.check_in(ns, "state::get", &json!({}), m),
                Decision::Allow { .. }
            ));
        }
        assert_eq!(
            p.check_in(Some("laptop"), "shell::exec", &json!({}), m),
            Decision::Allow {
                rule_id: "laptop/shell::*".into()
            }
        );
        assert_eq!(
            p.check_in(Some("desktop"), "shell::exec", &json!({}), m),
            Decision::NeedsApproval
        );
    }

    #[test]
    fn qualified_rules_fail_closed_for_a_call_without_namespace() {
        // old harness
        let p = Permissions::compile(&[
            RuleSpec::Shorthand("!laptop/shell::exec".into()),
            RuleSpec::Shorthand("laptop/state::get".into()),
        ])
        .unwrap();
        assert!(matches!(
            p.check_in(None, "shell::exec", &json!({}), PermissionMode::Manual),
            Decision::Deny { .. }
        ));
        assert_eq!(
            p.check_in(None, "state::get", &json!({}), PermissionMode::Manual),
            Decision::NeedsApproval
        );
    }

    #[test]
    fn a_qualifier_without_an_id_does_not_compile() {
        for (spec, message) in [
            (
                RuleSpec::Shorthand("!laptop/".into()),
                "deny shorthand must be !<function_id>",
            ),
            (
                RuleSpec::Shorthand("laptop/".into()),
                "allow shorthand must be <function_id>",
            ),
            (
                RuleSpec::Structured {
                    rule_id: None,
                    function: "laptop/".into(),
                    action: Action::Allow,
                    modes: None,
                    args: Vec::new(),
                },
                "function is required",
            ),
        ] {
            let err = Permissions::compile(&[spec]).unwrap_err().to_string();
            assert!(err.contains(message), "{err}");
        }
    }

    #[test]
    fn structured_rule_splits_function_and_keeps_the_qualifier_in_rule_id() {
        let p = Permissions::compile(&[RuleSpec::Structured {
            rule_id: None,
            function: "laptop/web::*".into(),
            action: Action::Deny,
            modes: None,
            args: Vec::new(),
        }])
        .unwrap();
        assert!(matches!(
            p.check_in(Some("laptop"), "web::fetch", &json!({}), PermissionMode::Manual),
            Decision::Deny { rule_id, .. } if rule_id == "laptop/web::*"
        ));
        assert_eq!(
            p.check_in(
                Some("desktop"),
                "web::fetch",
                &json!({}),
                PermissionMode::Manual
            ),
            Decision::NeedsApproval
        );
    }
}
