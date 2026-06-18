use serde_json::Value;

use crate::types::{MatchedConstraint, PermissionMode};

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
        /// When set, the rule applies only in these session modes. Omit for all modes.
        modes: Option<Vec<PermissionMode>>,
        args: Vec<(String, ConstraintSpec)>,
    },
}

#[derive(Debug, Clone)]
pub enum ConstraintSpec {
    Equals(Value),
    Matches(String),
}
