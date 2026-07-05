//! Built-in permission rules for step 5 of `approval::gate` when the
//! `approval-gate` configuration entry omits `rules` (or they fail to
//! compile). First match wins; no match → **hold**.
//!
//! The shipped defaults:
//! - deny **only this worker's `approval::*` surface** (the registered
//!   functions — see `functions::catalog`);
//! - allow the READ-ONLY coder surface in **auto mode only** (context /
//!   info / read-file / search / tree / list-folder), so an auto-mode
//!   coding session prompts on writes and exec, not on every read.
//!   These auto-scoped allows also feed `auto_allow_seed`. Manual mode
//!   is unaffected — mode-scoped rules do not match there.
//!
//! Every other function_id holds until the operator adds custom `rules`.

use serde_json::{json, Value};

/// The read-only coder functions trusted in auto mode by default.
pub const READ_ONLY_CODER: [&str; 6] = [
    "coder::context",
    "coder::info",
    "coder::read-file",
    "coder::search",
    "coder::tree",
    "coder::list-folder",
];

/// Shorthand rule strings compiled into [`super::default_permissions`].
pub fn default_rule_strings() -> Vec<&'static str> {
    vec!["!approval::*"]
}

/// The full default `rules` array as configuration JSON values:
/// shorthands plus the structured auto-scoped read-only coder allows.
pub fn default_rule_values() -> Vec<Value> {
    let mut out: Vec<Value> = default_rule_strings()
        .into_iter()
        .map(|s| Value::String(s.to_string()))
        .collect();
    for fid in READ_ONLY_CODER {
        out.push(json!({
            "rule_id": format!("seed-auto-{}", fid.replace("::", "-")),
            "function": fid,
            "action": "allow",
            "modes": ["auto"],
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::catalog;
    use crate::permissions::{default_permissions, Decision};
    use crate::types::PermissionMode;

    fn catalog_function_ids() -> Vec<&'static str> {
        catalog().iter().map(|s| s.function_id).collect()
    }

    #[test]
    fn default_rule_is_single_glob_over_registered_surface() {
        assert_eq!(default_rule_strings(), ["!approval::*"]);
    }

    #[test]
    fn default_values_add_auto_scoped_read_only_coder_allows() {
        let values = default_rule_values();
        assert_eq!(values.len(), 1 + READ_ONLY_CODER.len());
        for v in &values[1..] {
            assert_eq!(v["action"], "allow");
            assert_eq!(v["modes"], json!(["auto"]));
        }
    }

    #[test]
    fn read_only_coder_allows_in_auto_hold_in_manual() {
        use crate::permissions::{parse_rules_from_config, Permissions};
        let p = Permissions::compile(&parse_rules_from_config(&Value::Array(
            default_rule_values(),
        )))
        .unwrap();
        for fid in READ_ONLY_CODER {
            assert!(
                matches!(
                    p.check(fid, &json!({}), PermissionMode::Auto),
                    Decision::Allow { .. }
                ),
                "{fid} should allow in auto"
            );
            assert!(
                matches!(
                    p.check(fid, &json!({}), PermissionMode::Manual),
                    Decision::NeedsApproval
                ),
                "{fid} should still hold in manual"
            );
        }
        // Writes and exec stay gated even in auto.
        for fid in ["coder::update-file", "coder::apply-patch", "shell::exec"] {
            assert!(
                matches!(
                    p.check(fid, &json!({}), PermissionMode::Auto),
                    Decision::NeedsApproval
                ),
                "{fid} should hold in auto"
            );
        }
    }

    #[test]
    fn default_rules_deny_every_registered_function() {
        let p = default_permissions();
        for fid in catalog_function_ids() {
            assert!(
                matches!(
                    p.check(fid, &serde_json::json!({}), PermissionMode::Manual),
                    Decision::Deny { .. }
                ),
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
                matches!(
                    p.check(fid, &serde_json::json!({}), PermissionMode::Manual),
                    Decision::NeedsApproval
                ),
                "{fid} should hold"
            );
        }
    }
}
