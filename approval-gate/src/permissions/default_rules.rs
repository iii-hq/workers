//! Built-in permission rules for step 5 of `approval::gate` when the
//! `approval-gate` configuration entry omits `rules` (or they fail to
//! compile). First match wins; no match → **hold**.
//!
//! The shipped defaults deny **only this worker's `approval::*` surface**
//! (the 12 registered functions — see `functions::catalog`). Every
//! other function_id holds until the operator adds custom `rules`.

/// Shorthand rule strings compiled into [`super::default_permissions`].
pub fn default_rule_strings() -> Vec<&'static str> {
    vec!["!approval::*"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::catalog;
    use crate::permissions::{default_permissions, Decision};

    fn catalog_function_ids() -> Vec<&'static str> {
        catalog().iter().map(|s| s.function_id).collect()
    }

    #[test]
    fn default_rule_is_single_glob_over_registered_surface() {
        assert_eq!(default_rule_strings(), ["!approval::*"]);
    }

    #[test]
    fn default_rules_deny_every_registered_function() {
        let p = default_permissions();
        for fid in catalog_function_ids() {
            assert!(
                matches!(p.check(fid, &serde_json::json!({})), Decision::Deny { .. }),
                "{fid} should deny"
            );
        }
    }

    #[test]
    fn default_rules_hold_other_workers() {
        let p = default_permissions();
        for fid in [
            "shell::run",
            "web::fetch",
            "provider::anthropic::chat",
            "configuration::set",
        ] {
            assert!(
                matches!(p.check(fid, &serde_json::json!({})), Decision::NeedsApproval),
                "{fid} should hold"
            );
        }
    }
}
