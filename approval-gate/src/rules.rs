//! Layered permission rules — first-class policy primitive ported from
//! opencode's `Permission.evaluate` / `Wildcard.match`.
//!
//! ## Shape
//!
//! A [`Rule`] pairs a permission glob with a pattern glob over the call's
//! declared subject: shell argv for `shell::exec*`, declared filesystem paths
//! for `shell::fs::*`, and the canonical function id for non-shell tools.
//! [`Action::Allow`] passes the call through, [`Action::Deny`] hard-blocks,
//! and [`Action::Ask`] writes a pending approval row.
//!
//! ## Layering
//!
//! Global config is evaluated with **first match wins**. Per-session rules
//! created by `allow + always` are a guarded overlay: they may auto-allow
//! calls that would otherwise ask, but never override a matching global deny.
//!
//! ## Wildcard match
//!
//! [`wildcard_match`] supports `*` (zero or more of any character) and
//! literal text. No regex, no `?`, no character classes — the surface is
//! intentionally tiny to match opencode's `Wildcard.match` behaviour and
//! keep the rule language operator-readable. `*` is greedy via dynamic
//! programming so `"a*b*c"` matches `"axxxbxxxc"` correctly.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::path::Path;

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
/// `permission` is matched against the canonical iii function id (for example
/// `shell::exec`, `shell::fs::*`). `pattern` is matched against a subject
/// extracted from the call arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rule {
    /// Wildcard pattern matched against the iii function id.
    pub permission: String,
    /// Wildcard/glob pattern matched against argv, declared path, or function id.
    pub pattern: String,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleWire {
    permission: String,
    pattern: String,
    action: Action,
    #[serde(default)]
    reason: Option<String>,
}

impl TryFrom<RuleWire> for Rule {
    type Error = String;

    fn try_from(raw: RuleWire) -> Result<Self, Self::Error> {
        if raw.permission.is_empty() {
            return Err("rule.permission must not be empty; use \"*\" for wildcard".to_string());
        }
        if raw.pattern.is_empty() {
            return Err("rule.pattern must not be empty; use \"*\" for wildcard".to_string());
        }
        Ok(Self {
            permission: raw.permission,
            pattern: raw.pattern,
            action: raw.action,
            reason: raw.reason,
        })
    }
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        RuleWire::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// A list of rules, evaluated in order. The first matching rule wins.
pub type Ruleset = Vec<Rule>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleSource {
    Global,
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleMatch {
    pub source: RuleSource,
    pub index: usize,
    pub permission: String,
    pub pattern: String,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl RuleMatch {
    fn from_rule(source: RuleSource, index: usize, rule: &Rule) -> Self {
        Self {
            source,
            index,
            permission: rule.permission.clone(),
            pattern: rule.pattern.clone(),
            action: rule.action,
            reason: rule.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesSnapshot {
    pub global: Option<Ruleset>,
    pub session: Ruleset,
}

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
/// so `verdict_for` can apply the guarded session overlay without mutating
/// global policy.
#[derive(Debug, Clone, Default)]
pub struct LayeredRules {
    pub global: Option<Ruleset>,
    pub per_session: std::collections::HashMap<String, Ruleset>,
}

impl LayeredRules {
    /// Construct a [`LayeredRules`] from a global ruleset only. Used at
    /// worker startup when YAML config is the only source of rules.
    pub fn from_global(global: Option<Ruleset>) -> Self {
        Self {
            global,
            per_session: std::collections::HashMap::new(),
        }
    }

    /// Build the rules snapshot that applies to `session_id`: optional
    /// global rules plus that session's `allow + always` overlay.
    pub fn snapshot_for(&self, session_id: &str) -> RulesSnapshot {
        RulesSnapshot {
            global: self.global.clone(),
            session: self
                .per_session
                .get(session_id)
                .cloned()
                .unwrap_or_default(),
        }
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

/// Find the **first** rule in `rules` whose `permission` and `pattern`
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
        .next()
}

pub fn evaluate_context(
    rules: &Ruleset,
    source: RuleSource,
    ctx: &MatchContext,
) -> Option<RuleMatch> {
    rules
        .iter()
        .enumerate()
        .find(|(_, rule)| rule_matches_context(rule, ctx))
        .map(|(idx, rule)| RuleMatch::from_rule(source, idx, rule))
}

pub fn any_matching_deny(
    rules: &Ruleset,
    source: RuleSource,
    ctx: &MatchContext,
) -> Option<RuleMatch> {
    rules
        .iter()
        .enumerate()
        .find(|(_, rule)| rule.action == Action::Deny && rule_matches_context(rule, ctx))
        .map(|(idx, rule)| RuleMatch::from_rule(source, idx, rule))
}

fn rule_matches_context(rule: &Rule, ctx: &MatchContext) -> bool {
    let rule_permission = canonical_permission(&rule.permission);
    if !wildcard_match(&rule_permission, &ctx.permission) {
        return false;
    }
    if ctx.malformed {
        return rule.pattern == "*" && rule.action != Action::Allow;
    }
    match ctx.kind {
        MatchKind::Fs => match rule.action {
            Action::Deny => ctx
                .subjects
                .iter()
                .any(|subject| glob_matches_path(&rule.pattern, subject)),
            Action::Allow | Action::Ask => {
                !ctx.subjects.is_empty()
                    && ctx
                        .subjects
                        .iter()
                        .all(|subject| glob_matches_path(&rule.pattern, subject))
            }
        },
        MatchKind::Exec | MatchKind::Generic => ctx
            .subjects
            .iter()
            .any(|subject| wildcard_match(&rule.pattern, subject)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Exec,
    Fs,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchContext {
    pub permission: String,
    pub subjects: Vec<String>,
    pub kind: MatchKind,
    pub malformed: bool,
}

pub fn match_context(function_id: &str, args: &Value) -> MatchContext {
    let permission = canonical_permission(function_id);
    match permission.as_str() {
        "shell::exec" | "shell::exec_bg" => exec_context(&permission, args),
        id if id.starts_with("shell::fs::") => fs_context(id, args),
        _ => MatchContext {
            permission: permission.clone(),
            subjects: vec![permission],
            kind: MatchKind::Generic,
            malformed: false,
        },
    }
}

/// Exact pattern used by allow+always session overlays. Malformed calls
/// produce `None` so the overlay cannot accidentally widen into `*`.
pub fn pattern_for(function_id: &str, args: &Value) -> Option<String> {
    let ctx = match_context(function_id, args);
    if ctx.malformed {
        return None;
    }
    ctx.subjects.first().cloned()
}

pub fn canonical_permission(permission: &str) -> String {
    match permission {
        "fs::list" => "shell::fs::ls".to_string(),
        p if p.starts_with("fs::") => format!("shell::fs::{}", &p["fs::".len()..]),
        other => other.to_string(),
    }
}

fn exec_context(permission: &str, args: &Value) -> MatchContext {
    let Some(command) = args.get("command").and_then(Value::as_str) else {
        return malformed_context(permission, MatchKind::Exec);
    };
    let argv = match args.get("args") {
        None | Some(Value::Null) => match shell_words::split(command) {
            Ok(argv) => argv,
            Err(_) => return malformed_context(permission, MatchKind::Exec),
        },
        Some(Value::Array(arr)) => {
            let mut argv = vec![command.to_string()];
            for item in arr {
                let Some(s) = item.as_str() else {
                    return malformed_context(permission, MatchKind::Exec);
                };
                argv.push(s.to_string());
            }
            argv
        }
        Some(_) => return malformed_context(permission, MatchKind::Exec),
    };
    if argv.is_empty() {
        return malformed_context(permission, MatchKind::Exec);
    }
    let exact = argv.join(" ");
    let mut basename = argv.clone();
    if let Some(first) = basename.first_mut() {
        if let Some(name) = Path::new(first).file_name().and_then(|s| s.to_str()) {
            *first = name.to_string();
        }
    }
    let basename = basename.join(" ");
    let mut subjects = vec![exact];
    if subjects[0] != basename {
        subjects.push(basename);
    }
    MatchContext {
        permission: permission.to_string(),
        subjects,
        kind: MatchKind::Exec,
        malformed: false,
    }
}

fn fs_context(permission: &str, args: &Value) -> MatchContext {
    let mut subjects = Vec::new();
    let mut malformed = false;
    let mut push_string = |field: &str| match args.get(field).and_then(Value::as_str) {
        Some(v) => subjects.push(v.to_string()),
        None => malformed = true,
    };
    match permission {
        "shell::fs::mv" => {
            push_string("src");
            push_string("dst");
        }
        "shell::fs::sed" => {
            if let Some(files) = args.get("files") {
                let Some(arr) = files.as_array() else {
                    return malformed_context(permission, MatchKind::Fs);
                };
                for item in arr {
                    let Some(s) = item.as_str() else {
                        return malformed_context(permission, MatchKind::Fs);
                    };
                    subjects.push(s.to_string());
                }
            }
            if let Some(path) = args.get("path") {
                match path.as_str() {
                    Some(s) => subjects.push(s.to_string()),
                    None => malformed = true,
                }
            }
            if subjects.is_empty() {
                malformed = true;
            }
        }
        _ => push_string("path"),
    }
    MatchContext {
        permission: permission.to_string(),
        subjects,
        kind: MatchKind::Fs,
        malformed,
    }
}

fn malformed_context(permission: &str, kind: MatchKind) -> MatchContext {
    MatchContext {
        permission: permission.to_string(),
        subjects: Vec::new(),
        kind,
        malformed: true,
    }
}

fn glob_matches_path(pattern: &str, relpath: &str) -> bool {
    if pattern.contains('/') {
        glob_match(pattern, relpath)
    } else {
        let base = relpath.rsplit('/').next().unwrap_or(relpath);
        glob_match(pattern, base)
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    if let Some(rest) = pattern.strip_prefix("**/") {
        let base = path.rsplit('/').next().unwrap_or(path);
        return glob_match_simple(rest, base) || glob_match_simple(rest, path);
    }
    glob_match_simple(pattern, path)
}

fn glob_match_simple(pattern: &str, path: &str) -> bool {
    let p = pattern.as_bytes();
    let t = path.as_bytes();
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        let pc = p.get(pi).copied();
        let tc = t[ti];
        match pc {
            Some(b'*') => {
                star_pi = Some(pi);
                star_ti = ti;
                pi += 1;
            }
            Some(b'?') if tc != b'/' => {
                pi += 1;
                ti += 1;
            }
            Some(c) if c == tc => {
                pi += 1;
                ti += 1;
            }
            _ => {
                if let Some(sp) = star_pi {
                    if t[star_ti] == b'/' {
                        return false;
                    }
                    pi = sp + 1;
                    star_ti += 1;
                    ti = star_ti;
                } else {
                    return false;
                }
            }
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(permission: &str, pattern: &str, action: Action) -> Rule {
        Rule {
            permission: permission.to_string(),
            pattern: pattern.to_string(),
            action,
            reason: None,
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
    fn evaluate_first_wins_within_single_ruleset() {
        // Two matching rules in the same ruleset; the earlier one wins.
        let rs: Ruleset = vec![
            r("shell::*", "*", Action::Allow),
            r("shell::exec", "*", Action::Deny),
        ];
        let m = evaluate("shell::exec", "*", &rs).expect("match");
        assert_eq!(
            m.action,
            Action::Allow,
            "first matching rule must win so catch-all rules can safely sit last"
        );
    }

    #[test]
    fn evaluate_chained_iterators_stay_simple_first_match_only() {
        // `evaluate` is a low-level iterator helper: it does not implement
        // production global/session overlay semantics. Guarded session allow
        // is handled by verdict_for.
        let global: Ruleset = vec![r("*", "*", Action::Allow)];
        let session: Ruleset = vec![r("shell::exec", "*", Action::Deny)];
        let m = evaluate("shell::exec", "*", global.iter().chain(session.iter())).expect("match");
        assert_eq!(m.action, Action::Allow);

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
    fn rule_rejects_unknown_fields() {
        let err = serde_yaml::from_str::<Rule>(
            r#"{ permission: "shell::exec", pattern: "*", action: ask, typo: true }"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn rule_rejects_empty_permission_and_pattern() {
        let err = serde_yaml::from_str::<Rule>(r#"{ permission: "", pattern: "*", action: ask }"#)
            .unwrap_err();
        assert!(err.to_string().contains("permission"), "{err}");

        let err = serde_yaml::from_str::<Rule>(r#"{ permission: "*", pattern: "", action: ask }"#)
            .unwrap_err();
        assert!(err.to_string().contains("pattern"), "{err}");
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
        assert_eq!(pat.as_deref(), Some("git status"));
    }

    #[test]
    fn pattern_for_shell_exec_bg_joins_command_with_args() {
        let pat = pattern_for(
            "shell::exec_bg",
            &json!({"command": "tail", "args": ["-f", "/var/log/x"]}),
        );
        assert_eq!(pat.as_deref(), Some("tail -f /var/log/x"));
    }

    #[test]
    fn pattern_for_shell_exec_single_string_command_no_args() {
        // shell::exec supports the "command is a shell-words string" mode
        // (args: None). The pattern is just the command string.
        let pat = pattern_for("shell::exec", &json!({"command": "git status"}));
        assert_eq!(pat.as_deref(), Some("git status"));
    }

    #[test]
    fn pattern_for_shell_exec_empty_args_list_is_verbatim_program_only() {
        let pat = pattern_for("shell::exec", &json!({"command": "ls", "args": []}));
        assert_eq!(pat.as_deref(), Some("ls"));
    }

    #[test]
    fn pattern_for_shell_exec_missing_command_returns_none() {
        let pat = pattern_for("shell::exec", &json!({"args": ["foo"]}));
        assert_eq!(pat, None);
    }

    #[test]
    fn pattern_for_shell_exec_completely_malformed_args_returns_none() {
        let pat = pattern_for("shell::exec", &json!(null));
        assert_eq!(pat, None);
    }

    #[test]
    fn pattern_for_non_shell_function_id_returns_function_id() {
        let pat = pattern_for("http::fetch", &json!({"url": "https://x"}));
        assert_eq!(pat.as_deref(), Some("http::fetch"));
    }

    #[test]
    fn exec_args_presence_controls_shell_words_vs_verbatim_command() {
        let shell_words = match_context("shell::exec", &json!({"command": "echo 'hello world'"}));
        assert_eq!(shell_words.subjects, vec!["echo hello world"]);

        let present_args = match_context(
            "shell::exec",
            &json!({"command": "echo 'hello world'", "args": []}),
        );
        assert_eq!(
            present_args.subjects,
            vec!["echo 'hello world'"],
            "when args is present, command is treated as argv[0] verbatim instead of shell-split",
        );

        let shell_split_rule = vec![r("shell::exec", "echo hello world", Action::Allow)];
        assert!(evaluate_context(&shell_split_rule, RuleSource::Global, &shell_words).is_some());
        assert!(
            evaluate_context(&shell_split_rule, RuleSource::Global, &present_args).is_none(),
            "present args must not accidentally match the absent-args shell-split subject",
        );

        let verbatim_rule = vec![r("shell::exec", "echo 'hello world'", Action::Allow)];
        assert!(evaluate_context(&verbatim_rule, RuleSource::Global, &present_args).is_some());
    }

    #[test]
    fn exec_match_uses_shell_words_and_basename_subjects() {
        let split = match_context(
            "shell::exec",
            &json!({"command": "git commit -m 'hi there'"}),
        );
        assert_eq!(split.subjects, vec!["git commit -m hi there"]);

        let path_cmd = match_context(
            "shell::exec",
            &json!({"command": "/usr/bin/git", "args": ["status"]}),
        );
        assert_eq!(
            path_cmd.subjects,
            vec!["/usr/bin/git status", "git status"],
            "path-qualified exec subjects must keep both exact argv and basename-normalized argv",
        );

        let full_path = vec![r("shell::exec", "/usr/bin/git status", Action::Allow)];
        let m =
            evaluate_context(&full_path, RuleSource::Global, &path_cmd).expect("exact path match");
        assert_eq!(m.action, Action::Allow);

        let basename = vec![r("shell::exec", "git status", Action::Allow)];
        let m = evaluate_context(&basename, RuleSource::Global, &path_cmd).expect("basename match");
        assert_eq!(m.action, Action::Allow);
    }

    #[test]
    fn evaluate_context_supports_aliases_and_catch_all_last() {
        let rs = vec![r("fs::read", "*", Action::Allow), r("*", "*", Action::Ask)];
        let ctx = match_context("shell::fs::read", &json!({"path": "/tmp/a"}));
        let m = evaluate_context(&rs, RuleSource::Global, &ctx).expect("match");
        assert_eq!(m.action, Action::Allow);
        assert_eq!(m.index, 0);
    }

    #[test]
    fn fs_mv_allow_requires_all_paths_but_deny_matches_any() {
        let ctx = match_context(
            "shell::fs::mv",
            &json!({"src": "/safe/a", "dst": "/other/b"}),
        );
        let allow = vec![r("shell::fs::mv", "/safe/*", Action::Allow)];
        assert!(evaluate_context(&allow, RuleSource::Global, &ctx).is_none());

        let deny = vec![r("shell::fs::mv", "/safe/*", Action::Deny)];
        let m = evaluate_context(&deny, RuleSource::Global, &ctx).expect("deny matches src");
        assert_eq!(m.action, Action::Deny);
    }

    #[test]
    fn fs_declared_scope_handles_grep_and_sed_paths() {
        let grep = match_context("shell::fs::grep", &json!({"path": "src/lib.rs"}));
        let allow = vec![r("fs::grep", "src/*", Action::Allow)];
        assert_eq!(
            evaluate_context(&allow, RuleSource::Global, &grep)
                .expect("grep declared path")
                .action,
            Action::Allow
        );

        let sed = match_context(
            "shell::fs::sed",
            &json!({"files": ["src/lib.rs", "src/main.rs"], "path": "src/extra.rs"}),
        );
        let src_only = vec![r("shell::fs::sed", "src/*", Action::Ask)];
        assert_eq!(
            evaluate_context(&src_only, RuleSource::Global, &sed)
                .expect("all declared sed paths are under src")
                .action,
            Action::Ask
        );

        let one_file = vec![r("shell::fs::sed", "main.rs", Action::Allow)];
        assert!(
            evaluate_context(&one_file, RuleSource::Global, &sed).is_none(),
            "allow/ask must match all declared paths, not only one file"
        );
    }

    #[test]
    fn malformed_exec_matches_only_ask_or_deny_wildcard() {
        let ctx = match_context("shell::exec", &json!({"command": "git", "args": [3]}));
        assert!(ctx.malformed);
        let allow = vec![r("shell::exec", "*", Action::Allow)];
        assert!(evaluate_context(&allow, RuleSource::Global, &ctx).is_none());

        let specific_deny = vec![r("shell::exec", "git*", Action::Deny)];
        assert!(
            evaluate_context(&specific_deny, RuleSource::Global, &ctx).is_none(),
            "malformed argv has no trustworthy subject for specific patterns",
        );

        let ask = vec![r("shell::exec", "*", Action::Ask)];
        assert_eq!(
            evaluate_context(&ask, RuleSource::Global, &ctx)
                .expect("ask wildcard")
                .action,
            Action::Ask
        );

        let deny = vec![r("shell::exec", "*", Action::Deny)];
        assert_eq!(
            evaluate_context(&deny, RuleSource::Global, &ctx)
                .expect("deny wildcard")
                .action,
            Action::Deny
        );
    }

    #[test]
    fn malformed_fs_scope_matches_only_ask_or_deny_wildcard() {
        let ctx = match_context(
            "shell::fs::sed",
            &json!({"files": ["src/lib.rs", 3], "path": "src/main.rs"}),
        );
        assert!(ctx.malformed);

        let allow = vec![r("shell::fs::sed", "*", Action::Allow)];
        assert!(evaluate_context(&allow, RuleSource::Global, &ctx).is_none());

        let specific_deny = vec![r("shell::fs::sed", "src/*", Action::Deny)];
        assert!(
            evaluate_context(&specific_deny, RuleSource::Global, &ctx).is_none(),
            "malformed filesystem envelopes must not apply path-specific rules to partial data",
        );

        let ask = vec![r("shell::fs::sed", "*", Action::Ask)];
        assert_eq!(
            evaluate_context(&ask, RuleSource::Global, &ctx)
                .expect("ask wildcard")
                .action,
            Action::Ask
        );

        let deny = vec![r("shell::fs::sed", "*", Action::Deny)];
        assert_eq!(
            evaluate_context(&deny, RuleSource::Global, &ctx)
                .expect("deny wildcard")
                .action,
            Action::Deny
        );
    }
}
