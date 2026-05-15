//! Layered permission rules — first-class policy primitive ported from
//! opencode's `Permission.evaluate` / `Wildcard.match`.
//!
//! ## Shape
//!
//! A [`Rule`] pairs a permission glob (matched against the iii function id)
//! with a pattern glob (matched against a caller-supplied pattern string,
//! always `"*"` in v1 — see [`evaluate`] for the forward-compatible call
//! shape). An [`Action`] tells the gate what to do on match:
//! [`Action::Allow`] passes the call through, [`Action::Deny`] short-circuits
//! with a policy [`crate::Denial`], [`Action::Ask`] falls back to the existing
//! per-function [`crate::config::InterceptorRule`] flow.
//!
//! ## Layering
//!
//! Operators stack rules — a workspace-default ruleset, plus a per-session
//! override, plus an operator-pinned global. [`evaluate`] flattens N
//! rulesets in caller order and returns the **last** matching rule.
//! Last-wins is the standard policy-stacking semantic: a more-specific
//! later layer overrides an earlier permissive default without surgery on
//! the earlier list.
//!
//! ## Wildcard match
//!
//! [`wildcard_match`] supports `*` (zero or more of any character) and
//! literal text. No regex, no `?`, no character classes — the surface is
//! intentionally tiny to match opencode's `Wildcard.match` behaviour and
//! keep the rule language operator-readable. `*` is greedy via dynamic
//! programming so `"a*b*c"` matches `"axxxbxxxc"` correctly.

use serde::{Deserialize, Serialize};

/// Decision a [`Rule`] expresses when it matches an incoming call.
///
/// Wire format is the lowercase string `"allow"` | `"deny"` | `"ask"` so
/// rules are operator-readable in YAML / JSON config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
    Ask,
}

/// A single permission rule.
///
/// `permission` is matched against the iii function id (e.g. `shell::exec`,
/// `shell::fs::*`). `pattern` is matched against a caller-supplied pattern
/// string; in v1 every call site passes `"*"`, so `pattern: "*"` is the
/// only useful value today. The field is kept on the type so the forward
/// path to per-function pattern extractors (shell::exec → joined argv,
/// shell::fs::* → path) is a config-level change, not a schema break.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// Wildcard pattern matched against the iii function id.
    pub permission: String,
    /// Wildcard pattern matched against a caller-supplied pattern string.
    /// In v1 callers pass `"*"`; setting `pattern: "*"` here matches them.
    pub pattern: String,
    pub action: Action,
}

/// A list of rules, evaluated in order. Stacked rulesets are flattened by
/// [`evaluate`] in caller order so the **last** matching rule across all
/// layers wins.
pub type Ruleset = Vec<Rule>;

/// True if `text` matches the wildcard `pattern`. Supports `*` (zero or
/// more of any character) and literal text. Tiny on purpose — operators
/// should be able to read a rule and know what it matches without a regex
/// engine in their head.
///
/// Dynamic-programming implementation so `"a*b*c"` matches `"axxxbxxxc"`
/// without exponential backtracking on patterns with many `*`.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (np, nt) = (p.len(), t.len());
    // dp[i][j] = true iff p[..i] matches t[..j].
    let mut dp = vec![vec![false; nt + 1]; np + 1];
    dp[0][0] = true;
    // A leading run of '*' can match the empty string.
    for i in 1..=np {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=np {
        for j in 1..=nt {
            dp[i][j] = if p[i - 1] == '*' {
                // '*' matches empty (dp[i-1][j]) or extends by one char (dp[i][j-1]).
                dp[i - 1][j] || dp[i][j - 1]
            } else {
                p[i - 1] == t[j - 1] && dp[i - 1][j - 1]
            };
        }
    }
    dp[np][nt]
}

/// Find the **last** rule in `rules` whose `permission` and `pattern`
/// both wildcard-match the given inputs. Takes any iterator of rule
/// references so callers can pass a single [`Ruleset`] directly
/// (`&Vec<Rule>` is `IntoIterator<Item = &Rule>`) or chain several layers
/// via `global.iter().chain(session.iter())` without temporary borrows.
/// Returns the matched rule by reference so the caller can read its
/// [`Action`] and report the matching pattern in audit / Denial detail.
///
/// `None` means no rule matched — the caller should fall back to whatever
/// it would do without a rules layer (in approval-gate: the existing
/// per-function [`crate::config::InterceptorRule`] path).
pub fn evaluate<'a, I>(permission: &str, pattern: &str, rules: I) -> Option<&'a Rule>
where
    I: IntoIterator<Item = &'a Rule>,
{
    rules
        .into_iter()
        .filter(|r| wildcard_match(&r.permission, permission) && wildcard_match(&r.pattern, pattern))
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(permission: &str, pattern: &str, action: Action) -> Rule {
        Rule {
            permission: permission.to_string(),
            pattern: pattern.to_string(),
            action,
        }
    }

    #[test]
    fn wildcard_literal_match() {
        assert!(wildcard_match("shell::exec", "shell::exec"));
        assert!(!wildcard_match("shell::exec", "shell::fs::read"));
    }

    #[test]
    fn wildcard_star_matches_empty() {
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("*", "anything"));
    }

    #[test]
    fn wildcard_star_matches_prefix() {
        assert!(wildcard_match("shell::*", "shell::exec"));
        assert!(wildcard_match("shell::*", "shell::fs::write"));
        assert!(!wildcard_match("shell::*", "approval::resolve"));
    }

    #[test]
    fn wildcard_star_matches_suffix_and_middle() {
        assert!(wildcard_match("*::exec", "shell::exec"));
        assert!(wildcard_match("shell::*::write", "shell::fs::write"));
        assert!(!wildcard_match("shell::*::write", "shell::fs::read"));
    }

    #[test]
    fn wildcard_multiple_stars_no_backtracking_blowup() {
        // The dp implementation must not blow up on many '*'.
        let pat = "*a*a*a*a*a*a*a*a*a*a*a*a*a*b";
        let text: String = "a".repeat(50);
        assert!(!wildcard_match(pat, &text));
        let text_ok: String = format!("{}b", "a".repeat(50));
        assert!(wildcard_match(pat, &text_ok));
    }

    #[test]
    fn evaluate_returns_none_for_empty_ruleset() {
        let empty: Ruleset = vec![];
        assert!(evaluate("shell::exec", "*", &empty).is_none());
    }

    #[test]
    fn evaluate_returns_none_when_nothing_matches() {
        let rs: Ruleset = vec![r("approval::*", "*", Action::Allow)];
        assert!(evaluate("shell::exec", "*", &rs).is_none());
    }

    #[test]
    fn evaluate_matches_exact_permission() {
        let rs: Ruleset = vec![r("shell::exec", "*", Action::Allow)];
        let m = evaluate("shell::exec", "*", &rs).expect("match");
        assert_eq!(m.action, Action::Allow);
    }

    #[test]
    fn evaluate_matches_wildcard_permission() {
        let rs: Ruleset = vec![r("shell::*", "*", Action::Allow)];
        let m = evaluate("shell::fs::write", "*", &rs).expect("match");
        assert_eq!(m.action, Action::Allow);
    }

    #[test]
    fn evaluate_last_wins_within_single_ruleset() {
        // Two matching rules in the same ruleset; the later one wins.
        let rs: Ruleset = vec![
            r("shell::*", "*", Action::Allow),
            r("shell::exec", "*", Action::Deny),
        ];
        let m = evaluate("shell::exec", "*", &rs).expect("match");
        assert_eq!(
            m.action,
            Action::Deny,
            "more-specific later rule must override earlier permissive default"
        );
    }

    #[test]
    fn evaluate_last_wins_across_layered_rulesets() {
        // global allows everything; session denies shell::exec. Session
        // (passed last) overrides global.
        let global: Ruleset = vec![r("*", "*", Action::Allow)];
        let session: Ruleset = vec![r("shell::exec", "*", Action::Deny)];
        let m = evaluate(
            "shell::exec",
            "*",
            global.iter().chain(session.iter()),
        )
        .expect("match");
        assert_eq!(m.action, Action::Deny);

        // For a permission only matched by global, global still wins.
        let m2 = evaluate(
            "approval::resolve",
            "*",
            global.iter().chain(session.iter()),
        )
        .expect("match");
        assert_eq!(m2.action, Action::Allow);
    }

    #[test]
    fn evaluate_ask_is_a_valid_action() {
        let rs: Ruleset = vec![r("shell::exec", "*", Action::Ask)];
        let m = evaluate("shell::exec", "*", &rs).expect("match");
        assert_eq!(m.action, Action::Ask);
    }

    #[test]
    fn evaluate_pattern_matches_when_both_globs_pass() {
        let rs: Ruleset = vec![r("shell::exec", "git*", Action::Allow)];
        // pattern matches
        let m = evaluate("shell::exec", "git checkout main", &rs).expect("match");
        assert_eq!(m.action, Action::Allow);
        // pattern doesn't match → no rule selected
        assert!(evaluate("shell::exec", "rm -rf /", &rs).is_none());
    }

    #[test]
    fn rule_serde_round_trip() {
        let original = r("shell::exec", "*", Action::Deny);
        let json = serde_json::to_value(&original).unwrap();
        assert_eq!(json["permission"], "shell::exec");
        assert_eq!(json["pattern"], "*");
        assert_eq!(json["action"], "deny");
        let back: Rule = serde_json::from_value(json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn action_yaml_round_trip() {
        for a in [Action::Allow, Action::Deny, Action::Ask] {
            let y = serde_yaml::to_string(&a).unwrap();
            let back: Action = serde_yaml::from_str(&y).unwrap();
            assert_eq!(back, a);
        }
    }
}
