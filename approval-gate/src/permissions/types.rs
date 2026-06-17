use serde_json::Value;

use crate::types::MatchedConstraint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow {
        rule_id: String,
    },
    Deny {
        rule_id: String,
        matched_constraint: Option<MatchedConstraint>,
    },
    NeedsApproval,
}

/// One entry from the `rules` configuration array.
#[derive(Debug, Clone)]
pub enum RuleSpec {
    Shorthand(String),
    Structured {
        rule_id: Option<String>,
        function: String,
        action: Action,
        args: Vec<(String, ConstraintSpec)>,
    },
}

#[derive(Debug, Clone)]
pub enum ConstraintSpec {
    Equals(Value),
    Matches(String),
}
