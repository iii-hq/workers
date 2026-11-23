use regex::Regex;

use super::split_namespace_pattern;
use super::types::{Action, ConstraintSpec, RuleSpec};

#[derive(Debug, Clone)]
pub enum CompiledConstraint {
    Equals(serde_json::Value),
    Matches { regex: Regex, pattern: String },
}

#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub rule_id: String,
    /// The `<namespace>/` qualifier; `None` matches every namespace.
    pub namespace: Option<String>,
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
        let CompileError::Rule { index: _, message } = &mut e;
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

/// A glob, empty or doubled-slash qualifier would silently match nothing (`!*/x` and `!/x`
/// denied nothing).
fn check_namespace_qualifier(
    pattern: &str,
    namespace: Option<&str>,
    index: usize,
) -> Result<(), CompileError> {
    let bad = match namespace {
        None if pattern.starts_with('/') => {
            Some(("", "drop the leading `/` to match every namespace"))
        }
        Some(ns) if ns.contains(['*', '?', '[', '{']) => {
            Some((ns, "a bare pattern already matches every namespace"))
        }
        // `laptop//x` splits to the literal id `/x`, which no call carries.
        Some(ns) if pattern[ns.len() + 1..].starts_with('/') => Some((
            &pattern[..ns.len() + 2],
            "put a single `/` between the namespace and the function id",
        )),
        _ => None,
    };
    match bad {
        Some((ns, hint)) => Err(CompileError::Rule {
            index,
            message: format!("namespace qualifier must be a plain namespace, got {ns:?}; {hint}"),
        }),
        None => Ok(()),
    }
}

/// A deny in the old form would silently match nothing.
fn check_old_mesh_prefix(id: &str, index: usize) -> Result<(), CompileError> {
    if !id.starts_with("hub::") && !id.starts_with("peer::") {
        return Ok(());
    }
    Err(CompileError::Rule {
        index,
        message: format!(
            "`{id}` uses a mesh id prefix that no longer exists. A mesh function keeps its id and lives in the namespace of its machine: write `<namespace>/<function id>`. `peer::laptop::shell::exec` becomes `laptop/shell::exec`, `hub::shell::exec` becomes `hub/shell::exec`, and `peer::*` becomes one rule per machine, such as `laptop/*`"
        ),
    })
}

pub fn compile_rule(spec: &RuleSpec, index: usize) -> Result<CompiledRule, CompileError> {
    match spec {
        RuleSpec::Shorthand(raw) => {
            let (pattern, action, empty_message) = match raw.strip_prefix('!') {
                Some(pattern) => (
                    pattern,
                    Action::Deny,
                    "deny shorthand must be !<function_id>",
                ),
                None => (
                    raw.as_str(),
                    Action::Allow,
                    "allow shorthand must be <function_id>",
                ),
            };
            let (namespace, id) = split_namespace_pattern(pattern);
            check_namespace_qualifier(pattern, namespace, index)?;
            check_old_mesh_prefix(id, index)?;
            // A bare empty allow `""` compiles as it always did.
            if id.is_empty() && (namespace.is_some() || action == Action::Deny) {
                return Err(CompileError::Rule {
                    index,
                    message: empty_message.into(),
                });
            }
            let (function_id, glob) = compile_function_matcher(id, index)?;
            Ok(CompiledRule {
                rule_id: pattern.to_string(),
                namespace: namespace.map(str::to_string),
                function_id,
                glob,
                action,
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
            let (namespace, id) = split_namespace_pattern(function);
            check_namespace_qualifier(function, namespace, index)?;
            check_old_mesh_prefix(id, index)?;
            if id.is_empty() {
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
            let (function_id, glob) = compile_function_matcher(id, index)?;
            let mut constraints: Vec<(String, CompiledConstraint)> = args
                .iter()
                .map(|(field, c)| compile_constraint(&rid, field, c).map(|cc| (field.clone(), cc)))
                .collect::<Result<Vec<_>, _>>()?;
            constraints.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(CompiledRule {
                rule_id: rid,
                namespace: namespace.map(str::to_string),
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
