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
use serde_json::Value;

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

/// Workspace-global rules + an overlay of per-session rules.
///
/// This is the runtime policy container the gate carries between intercept
/// and resolve. The `global` ruleset is the operator-configured baseline
/// (loaded from YAML) and applies to every session. The `per_session` map
/// holds short-lived rules pushed at runtime — today the only producer is
/// the `allow + always` cascade in `resolve::handle_resolve`, which scopes
/// the auto-allow to the originating session_id so a click in one chat
/// cannot silently bypass approval prompts in another.
///
/// Snapshots taken via [`LayeredRules::snapshot_for`] are flat `Ruleset`s
/// so existing pure helpers ([`evaluate`], [`crate::verdict_for`]) need no
/// changes to consume them.
#[derive(Debug, Clone, Default)]
pub struct LayeredRules {
    pub global: Ruleset,
    pub per_session: std::collections::HashMap<String, Ruleset>,
}

impl LayeredRules {
    /// Construct a [`LayeredRules`] from a global ruleset only. Used at
    /// worker startup when YAML config is the only source of rules.
    pub fn from_global(global: Ruleset) -> Self {
        Self {
            global,
            per_session: std::collections::HashMap::new(),
        }
    }

    /// Build the flat ruleset that applies to `session_id`: the global
    /// rules followed by the session's overlay (last-match-wins, so the
    /// overlay can override the global). Sessions with no overlay see
    /// only `global`.
    pub fn snapshot_for(&self, session_id: &str) -> Ruleset {
        let mut out = self.global.clone();
        if let Some(extra) = self.per_session.get(session_id) {
            out.extend(extra.iter().cloned());
        }
        out
    }

    /// Append a rule to the per-session overlay. The rule applies ONLY
    /// to that session — calls in other sessions will not see it.
    pub fn push_session_rule(&mut self, session_id: &str, rule: Rule) {
        self.per_session
            .entry(session_id.to_string())
            .or_default()
            .push(rule);
    }
}

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
        .filter(|r| {
            wildcard_match(&r.permission, permission) && wildcard_match(&r.pattern, pattern)
        })
        .last()
}

/// Per-function pattern extractor. The pattern is the second axis a rule
/// matches on (alongside `function_id`); for `shell::exec` we derive it
/// from `{command, args}` so operators can write rules like
/// `permission: "shell::exec", pattern: "git status*"` and get
/// argv-level granularity. Other function ids default to `"*"`, which
/// matches only wildcard rules.
pub fn pattern_for(function_id: &str, args: &Value) -> String {
    match function_id {
        "shell::exec" | "shell::exec_bg" => extract_shell_pattern(args),
        _ => "*".to_string(),
    }
}

/// Shell ExecRequest is `{ command: String, args: Option<Vec<String>> }`
/// per `shell/src/functions/types.rs`. There is no `argv` field. Two
/// modes:
///   - `args = None` → `command` is a shell-words string, use as-is.
///   - `args = Some(list)` → join `command + " " + list.join(" ")`.
/// Malformed input (missing/non-string command) falls back to `"*"` so
/// the row matches only wildcard rules.
///
/// Known conflation: argv `[git, log, "--grep=foo bar"]` joins to
/// `"git log --grep=foo bar"`, same pattern string as
/// `[git, log, "--grep=foo", bar]`. Documented; acceptable for v1.
fn extract_shell_pattern(args: &Value) -> String {
    let cmd = args.get("command").and_then(Value::as_str);
    let argv = args.get("args").and_then(Value::as_array);
    match (cmd, argv) {
        (Some(c), Some(arr)) if !arr.is_empty() => {
            let mut parts = vec![c.to_string()];
            parts.extend(arr.iter().filter_map(Value::as_str).map(str::to_string));
            parts.join(" ")
        }
        (Some(c), _) => c.to_string(),
        _ => "*".to_string(),
    }
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
        let m = evaluate("shell::exec", "*", global.iter().chain(session.iter())).expect("match");
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

    // -------------------- pattern_for / extract_shell_pattern --------------------

    use serde_json::json;

    #[test]
    fn pattern_for_shell_exec_joins_command_with_args() {
        let pat = pattern_for(
            "shell::exec",
            &json!({"command": "git", "args": ["status"]}),
        );
        assert_eq!(pat, "git status");
    }

    #[test]
    fn pattern_for_shell_exec_bg_joins_command_with_args() {
        let pat = pattern_for(
            "shell::exec_bg",
            &json!({"command": "tail", "args": ["-f", "/var/log/x"]}),
        );
        assert_eq!(pat, "tail -f /var/log/x");
    }

    #[test]
    fn pattern_for_shell_exec_single_string_command_no_args() {
        // shell::exec supports the "command is a shell-words string" mode
        // (args: None). The pattern is just the command string.
        let pat = pattern_for("shell::exec", &json!({"command": "git status"}));
        assert_eq!(pat, "git status");
    }

    #[test]
    fn pattern_for_shell_exec_empty_args_list_treated_as_no_args() {
        let pat = pattern_for("shell::exec", &json!({"command": "ls", "args": []}));
        assert_eq!(pat, "ls");
    }

    #[test]
    fn pattern_for_shell_exec_missing_command_falls_back_to_star() {
        let pat = pattern_for("shell::exec", &json!({"args": ["foo"]}));
        assert_eq!(pat, "*");
    }

    #[test]
    fn pattern_for_shell_exec_completely_malformed_args_falls_back_to_star() {
        let pat = pattern_for("shell::exec", &json!(null));
        assert_eq!(pat, "*");
    }

    #[test]
    fn pattern_for_non_shell_function_id_returns_star() {
        let pat = pattern_for("http::fetch", &json!({"url": "https://x"}));
        assert_eq!(pat, "*");
    }

    #[test]
    fn pattern_for_known_conflation_documented() {
        // Documented in spec: an arg containing a space conflates with two
        // separate args. This is acceptable for v1.
        let with_inner_space = pattern_for(
            "shell::exec",
            &json!({"command": "git", "args": ["log", "--grep=foo bar"]}),
        );
        let split_args = pattern_for(
            "shell::exec",
            &json!({"command": "git", "args": ["log", "--grep=foo", "bar"]}),
        );
        assert_eq!(
            with_inner_space, split_args,
            "v1 conflates space-in-arg with arg boundary; see spec"
        );
    }
}
