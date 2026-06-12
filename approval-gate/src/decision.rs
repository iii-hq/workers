//! The pure permission-model evaluation — no I/O. The order is ported
//! EXACTLY from the proven implementation
//! (harness/src/turn-orchestrator/hook.ts `consultBefore`, exercised by
//! mode-approval.e2e.test.ts) and the spec flowchart
//! (approval-gate.md § Evaluation order):
//!
//!   1. human-only target → deny (before the snapshot, even under full)
//!   2. mode full → allow
//!   3. approved_always hit → allow (every mode)
//!   4. mode auto AND always_allow hit → allow (dormant under manual)
//!   5. fall through to the yaml policy (policy.rs)

use crate::types::{AlwaysAllowEntry, ApprovalSettings, PermissionMode};

/// `approval::*` and `configuration::*` are operator surfaces — an agent
/// that could call them would approve its own calls (spec § Human-only
/// defense). Prefix match deliberately broadens the prior art's
/// six-function list.
pub fn is_human_only(function_id: &str) -> bool {
    function_id.starts_with("approval::") || function_id.starts_with("configuration::")
}

/// `*`-glob match with an equality fast-path. Globs exist because
/// `always_allow_seed` entries are documented as "function ids / globs"
/// (approval-gate.md § Configuration); plain entries behave exactly like
/// the prior art's equality test.
pub fn glob_match(pattern: &str, candidate: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == candidate;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, rest) = parts.split_first().expect("split always yields one part");
    if !candidate.starts_with(first) {
        return false;
    }
    let mut pos = first.len();
    for (i, part) in rest.iter().enumerate() {
        let is_last = i == rest.len() - 1;
        if part.is_empty() {
            continue;
        }
        if is_last {
            return candidate.len() >= pos + part.len() && candidate.ends_with(part);
        }
        match candidate[pos..].find(part) {
            Some(idx) => pos += idx + part.len(),
            None => return false,
        }
    }
    // The pattern ends with '*' (or was all '*'s): everything after the
    // anchored prefix matches.
    true
}

fn list_matches(entries: &[AlwaysAllowEntry], function_id: &str) -> bool {
    entries
        .iter()
        .any(|entry| glob_match(&entry.function_id, function_id))
}

/// Steps 2-4: the pre-policy short-circuits over one settings snapshot.
/// `false` = no short-circuit — fall through to the yaml policy.
pub fn pre_policy_allow(settings: &ApprovalSettings, function_id: &str) -> bool {
    if settings.mode == PermissionMode::Full {
        return true;
    }
    // Per-session "approve always" grants apply in every mode — they are
    // remembered human decisions, not an auto-policy.
    if list_matches(&settings.approved_always, function_id) {
        return true;
    }
    if settings.mode == PermissionMode::Auto && list_matches(&settings.always_allow, function_id) {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GrantedBy;

    fn entry(function_id: &str) -> AlwaysAllowEntry {
        AlwaysAllowEntry {
            function_id: function_id.to_string(),
            granted_at: 0,
            granted_by: GrantedBy::UserClick,
        }
    }

    fn settings(
        mode: PermissionMode,
        always_allow: &[&str],
        approved_always: &[&str],
    ) -> ApprovalSettings {
        ApprovalSettings {
            mode,
            always_allow: always_allow.iter().map(|f| entry(f)).collect(),
            approved_always: approved_always.iter().map(|f| entry(f)).collect(),
            mode_set_at: 0,
        }
    }

    #[test]
    fn human_only_covers_approval_and_configuration_prefixes() {
        assert!(is_human_only("approval::set_mode"));
        assert!(is_human_only("approval::resolve"));
        assert!(is_human_only("configuration::set"));
        assert!(!is_human_only("shell::run"));
        assert!(!is_human_only("approvals::other"));
    }

    #[test]
    fn full_mode_allows_everything() {
        assert!(pre_policy_allow(
            &settings(PermissionMode::Full, &[], &[]),
            "shell::run"
        ));
    }

    #[test]
    fn approved_always_holds_in_every_mode() {
        for mode in [
            PermissionMode::Manual,
            PermissionMode::Auto,
            PermissionMode::Full,
        ] {
            assert!(pre_policy_allow(
                &settings(mode, &[], &["shell::run"]),
                "shell::run"
            ));
        }
    }

    #[test]
    fn always_allow_works_only_in_auto() {
        assert!(pre_policy_allow(
            &settings(PermissionMode::Auto, &["shell::run"], &[]),
            "shell::run"
        ));
        // Dormant under manual: the user can build the list up, but it
        // only takes effect when they opt into auto.
        assert!(!pre_policy_allow(
            &settings(PermissionMode::Manual, &["shell::run"], &[]),
            "shell::run"
        ));
    }

    #[test]
    fn auto_mode_miss_falls_through() {
        assert!(!pre_policy_allow(
            &settings(PermissionMode::Auto, &["other::fn"], &[]),
            "shell::run"
        ));
    }

    #[test]
    fn manual_mode_with_no_grants_falls_through() {
        assert!(!pre_policy_allow(
            &settings(PermissionMode::Manual, &[], &[]),
            "shell::run"
        ));
    }

    #[test]
    fn glob_equality_fast_path() {
        assert!(glob_match("shell::run", "shell::run"));
        assert!(!glob_match("shell::run", "shell::runner"));
    }

    #[test]
    fn glob_star_patterns() {
        assert!(glob_match("shell::*", "shell::run"));
        assert!(glob_match("shell::*", "shell::"));
        assert!(!glob_match("shell::*", "state::get"));
        assert!(glob_match("*", "anything::at_all"));
        assert!(glob_match("*::get", "state::get"));
        assert!(!glob_match("*::get", "state::set"));
        assert!(glob_match("a*b*c", "a-x-b-y-c"));
        assert!(!glob_match("a*b*c", "a-x-c-y-b"));
        assert!(glob_match("state::*::read", "state::users::read"));
    }

    #[test]
    fn glob_entries_in_allow_lists_match() {
        assert!(pre_policy_allow(
            &settings(PermissionMode::Auto, &["shell::*"], &[]),
            "shell::run"
        ));
        assert!(pre_policy_allow(
            &settings(PermissionMode::Manual, &[], &["state::*"]),
            "state::get"
        ));
    }
}
