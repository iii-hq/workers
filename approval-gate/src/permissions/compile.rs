use regex::Regex;

use super::types::{Action, ConstraintSpec, RuleSpec};

#[derive(Debug, Clone)]
pub enum CompiledConstraint {
    Equals(serde_json::Value),
    Matches { regex: Regex, pattern: String },
}

#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub rule_id: String,
    pub function_id: String,
    pub glob: Option<Regex>,
    pub action: Action,
    /// When set, the rule applies only in these session modes.
    pub modes: Option<Vec<crate::types::PermissionMode>>,
    pub constraints: Vec<(String, CompiledConstraint)>,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("rule {index}: {message}")]
    Rule { index: usize, message: String },
}

fn compile_function_glob(pattern: &str) -> Result<Regex, CompileError> {
    let mut re = String::from('^');
    for ch in pattern.chars() {
        if ch == '*' {
            re.push_str(".*");
        } else if ".^$|?+()[]{}\\".contains(ch) {
            re.push('\\');
            re.push(ch);
        } else {
            re.push(ch);
        }
    }
    re.push('$');
    Regex::new(&re).map_err(|e| CompileError::Rule {
        index: 0,
        message: format!("invalid glob {pattern:?}: {e}"),
    })
}

fn compile_function_matcher(
    pattern: &str,
    index: usize,
) -> Result<(String, Option<Regex>), CompileError> {
    if !pattern.contains('*') {
        return Ok((pattern.to_string(), None));
    }
    let glob = compile_function_glob(pattern).map_err(|mut e| {
        let CompileError::Rule {
            index: _,
            ref mut message,
        } = &mut e;
        *message = format!("rule {index}: {message}");
        e
    })?;
    Ok((pattern.to_string(), Some(glob)))
}

pub fn match_function_id(rule: &CompiledRule, function_id: &str) -> bool {
    match &rule.glob {
        Some(glob) => glob.is_match(function_id),
        None => rule.function_id == function_id,
    }
}

fn compile_constraint(
    rule_id: &str,
    field: &str,
    spec: &ConstraintSpec,
) -> Result<CompiledConstraint, CompileError> {
    match spec {
        ConstraintSpec::Equals(v) => Ok(CompiledConstraint::Equals(v.clone())),
        ConstraintSpec::Matches(pattern) => {
            let regex = Regex::new(pattern).map_err(|e| CompileError::Rule {
                index: 0,
                message: format!("rule {rule_id}: invalid regex {pattern:?} on field {field}: {e}"),
            })?;
            Ok(CompiledConstraint::Matches {
                regex,
                pattern: pattern.clone(),
            })
        }
    }
}

pub fn compile_rule(spec: &RuleSpec, index: usize) -> Result<CompiledRule, CompileError> {
    match spec {
        RuleSpec::Shorthand(raw) => {
            if raw.starts_with('!') {
                let function_id = raw.strip_prefix('!').unwrap_or(raw);
                if function_id.is_empty() {
                    return Err(CompileError::Rule {
                        index,
                        message: "deny shorthand must be !<function_id>".into(),
                    });
                }
                let (function_id, glob) = compile_function_matcher(function_id, index)?;
                return Ok(CompiledRule {
                    rule_id: function_id.clone(),
                    function_id,
                    glob,
                    action: Action::Deny,
                    modes: None,
                    constraints: Vec::new(),
                });
            }
            let (function_id, glob) = compile_function_matcher(raw, index)?;
            Ok(CompiledRule {
                rule_id: function_id.clone(),
                function_id,
                glob,
                action: Action::Allow,
                modes: None,
                constraints: Vec::new(),
            })
        }
        RuleSpec::Structured {
            rule_id,
            function,
            action,
            modes,
            args,
        } => {
            if function.is_empty() {
                return Err(CompileError::Rule {
                    index,
                    message: "function is required".into(),
                });
            }
            let rid = rule_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(function.as_str())
                .to_string();
            let (function_id, glob) = compile_function_matcher(function, index)?;
            let mut constraints: Vec<(String, CompiledConstraint)> = args
                .iter()
                .map(|(field, c)| compile_constraint(&rid, field, c).map(|cc| (field.clone(), cc)))
                .collect::<Result<Vec<_>, _>>()?;
            constraints.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(CompiledRule {
                rule_id: rid,
                function_id,
                glob,
                action: *action,
                modes: modes.clone(),
                constraints,
            })
        }
    }
}

fn args_object(args: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    args.as_object()
}

fn constraint_matches(actual: &serde_json::Value, c: &CompiledConstraint) -> bool {
    match c {
        CompiledConstraint::Equals(expected) => actual == expected,
        CompiledConstraint::Matches { regex, .. } => {
            actual.as_str().is_some_and(|s| regex.is_match(s))
        }
    }
}

fn matched_constraint(field: &str, c: &CompiledConstraint) -> crate::types::MatchedConstraint {
    match c {
        CompiledConstraint::Equals(v) => crate::types::MatchedConstraint {
            field: field.to_string(),
            operator: "equals".to_string(),
            value: v.clone(),
        },
        CompiledConstraint::Matches { pattern, .. } => crate::types::MatchedConstraint {
            field: field.to_string(),
            operator: "matches".to_string(),
            value: serde_json::Value::String(pattern.clone()),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintMatch {
    Mismatch,
    NoConstraint,
    With,
}

pub fn match_constraints(
    args: &serde_json::Value,
    constraints: &[(String, CompiledConstraint)],
) -> (ConstraintMatch, Option<crate::types::MatchedConstraint>) {
    if constraints.is_empty() {
        return (ConstraintMatch::NoConstraint, None);
    }
    let Some(obj) = args_object(args) else {
        return (ConstraintMatch::Mismatch, None);
    };
    let mut matched: Option<crate::types::MatchedConstraint> = None;
    for (field, c) in constraints {
        let actual = obj.get(field).unwrap_or(&serde_json::Value::Null);
        if !constraint_matches(actual, c) {
            return (ConstraintMatch::Mismatch, None);
        }
        if matched.is_none() {
            matched = Some(matched_constraint(field, c));
        }
    }
    if matched.is_some() {
        (ConstraintMatch::With, matched)
    } else {
        (ConstraintMatch::NoConstraint, None)
    }
}
