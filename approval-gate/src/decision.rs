//! The pure permission-model evaluation — no I/O. The order is ported
//! EXACTLY from the proven implementation
//! (harness/src/turn-orchestrator/hook.ts `consultBefore`, exercised by
//! mode-approval.e2e.test.ts) and the spec flowchart
//! (approval-gate.md § Evaluation order):
//!
//!   1. human-only target → deny (before the snapshot, even under full)
//!   2. mode full → allow
//!   3. approved_always hit → allow (every mode; exact id, never a glob)
//!   4. mode auto AND always_allow hit → allow (dormant under manual; globs)
//!   5. fall through to configuration `rules` (permissions module)

use crate::types::{AlwaysAllowEntry, ApprovalSettings, PermissionMode};

/// `approval::*` and `configuration::*` are operator surfaces — an agent
/// that could call them would approve its own calls (spec § Human-only
/// defense). Prefix match deliberately broadens the prior art's
/// six-function list.
pub fn is_human_only(function_id: &str) -> bool {
    function_id.starts_with("approval::")
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

/// A grant matches its own namespace only. One stored without a namespace
/// covers the call's implicit namespace, so it keeps meaning what it meant
/// when it was granted.
fn list_matches(
    entries: &[AlwaysAllowEntry],
    id_matches: fn(&str, &str) -> bool,
    namespace: Option<&str>,
    implicit_namespace: Option<&str>,
    function_id: &str,
) -> bool {
    entries.iter().any(|entry| {
        id_matches(&entry.function_id, function_id)
            && match (entry.namespace.as_deref(), namespace) {
                (Some(granted), call) => call == Some(granted),
                (None, None) => true, // old harness: no namespace on the wire
                (None, call) => call == implicit_namespace, // stored before namespaces: the no-namespace route only
            }
    })
}

/// Steps 2-4: the pre-rules short-circuits over one settings snapshot.
/// `false` = no short-circuit — fall through to configuration rules.
pub fn pre_policy_allow(
    settings: &ApprovalSettings,
    namespace: Option<&str>,
    implicit_namespace: Option<&str>,
    function_id: &str,
) -> bool {
    if settings.mode == PermissionMode::Full {
        return true;
    }
    // Per-session "approve always" grants apply in every mode — they are
    // remembered human decisions, not an auto-policy. Exact, because the
    // prompt stores the held call's id as it arrived: a `*` the model put in
    // it must not widen what the human approved.
    if list_matches(
        &settings.approved_always,
        |granted, id| granted == id,
        namespace,
        implicit_namespace,
        function_id,
    ) {
        return true;
    }
    if settings.mode == PermissionMode::Auto
        && list_matches(
            &settings.always_allow,
            glob_match,
            namespace,
            implicit_namespace,
            function_id,
        )
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GrantedBy;

    fn entry(namespace: Option<&str>, function_id: &str) -> AlwaysAllowEntry {
        AlwaysAllowEntry {
            namespace: namespace.map(str::to_string),
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
            always_allow: always_allow.iter().map(|f| entry(None, f)).collect(),
            approved_always: approved_always.iter().map(|f| entry(None, f)).collect(),
            mode_set_at: 0,
        }
    }

    fn manual(entries: Vec<AlwaysAllowEntry>) -> ApprovalSettings {
        ApprovalSettings {
            mode: PermissionMode::Manual,
            approved_always: entries,
            ..Default::default()
        }
    }

    #[test]
    fn legacy_grant_covers_only_the_implicit_namespace() {
        let s = manual(vec![entry(None, "shell::run")]);
        assert!(pre_policy_allow(
            &s,
            Some("proj"),
            Some("proj"),
            "shell::run"
        ));
        assert!(!pre_policy_allow(
            &s,
            Some("laptop"),
            Some("proj"),
            "shell::run"
        ));
        assert!(!pre_policy_allow(&s, Some("laptop"), None, "shell::run"));
        assert!(pre_policy_allow(&s, None, None, "shell::run")); // old harness: as today
    }

    #[test]
    fn legacy_grant_on_a_pinned_id_still_matches_from_a_project_namespace() {
        // A harness in `proj` routes configuration::* to `default`, so both fields say `default`.
        let s = manual(vec![entry(None, "configuration::set")]);
        assert!(pre_policy_allow(
            &s,
            Some("default"),
            Some("default"),
            "configuration::set"
        ));
        assert!(!pre_policy_allow(
            &s,
            Some("proj"),
            Some("default"),
            "configuration::set"
        ));
    }

    #[test]
    fn namespaced_grant_matches_exactly_its_namespace() {
        let s = manual(vec![entry(Some("laptop"), "shell::run")]);
        assert!(pre_policy_allow(
            &s,
            Some("laptop"),
            Some("default"),
            "shell::run"
        ));
        assert!(!pre_policy_allow(
            &s,
            Some("desktop"),
            Some("default"),
            "shell::run"
        ));
        assert!(!pre_policy_allow(&s, None, None, "shell::run"));
    }

    #[test]
    fn approved_always_is_exact_not_a_pattern() {
        for mode in [PermissionMode::Manual, PermissionMode::Auto] {
            let s = settings(mode, &[], &["shell::*"]);
            assert!(!pre_policy_allow(&s, None, None, "shell::exec"));
            assert!(pre_policy_allow(&s, None, None, "shell::*"));
        }
    }

    #[test]
    fn glob_always_allow_stays_in_its_namespace() {
        let s = ApprovalSettings {
            mode: PermissionMode::Auto,
            always_allow: vec![entry(Some("laptop"), "shell::*")],
            ..Default::default()
        };
        assert!(pre_policy_allow(
            &s,
            Some("laptop"),
            Some("default"),
            "shell::exec"
        ));
        assert!(!pre_policy_allow(
            &s,
            Some("default"),
            Some("default"),
            "shell::exec"
        ));
        assert!(!pre_policy_allow(&s, None, Some("default"), "shell::exec"));
    }

    #[test]
    fn human_only_covers_approval_prefix() {
        assert!(is_human_only("approval::set-mode"));
        assert!(is_human_only("approval::resolve"));
        assert!(!is_human_only("configuration::set"));
        assert!(!is_human_only("shell::run"));
        assert!(!is_human_only("approvals::other"));
    }

    #[test]
    fn full_mode_allows_everything() {
        assert!(pre_policy_allow(
            &settings(PermissionMode::Full, &[], &[]),
            None,
            None,
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
                None,
                None,
                "shell::run"
            ));
        }
    }

    #[test]
    fn always_allow_works_only_in_auto() {
        assert!(pre_policy_allow(
            &settings(PermissionMode::Auto, &["shell::run"], &[]),
            None,
            None,
            "shell::run"
        ));
        // Dormant under manual: the user can build the list up, but it
        // only takes effect when they opt into auto.
        assert!(!pre_policy_allow(
            &settings(PermissionMode::Manual, &["shell::run"], &[]),
            None,
            None,
            "shell::run"
        ));
    }

    #[test]
    fn auto_mode_miss_falls_through() {
        assert!(!pre_policy_allow(
            &settings(PermissionMode::Auto, &["other::fn"], &[]),
            None,
            None,
            "shell::run"
        ));
    }

    #[test]
    fn manual_mode_with_no_grants_falls_through() {
        assert!(!pre_policy_allow(
            &settings(PermissionMode::Manual, &[], &[]),
            None,
            None,
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
    fn glob_entries_in_always_allow_match() {
        assert!(pre_policy_allow(
            &settings(PermissionMode::Auto, &["shell::*"], &[]),
            None,
            None,
            "shell::run"
        ));
    }
}
