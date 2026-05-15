//! `shell::classify_argv` — classifier path for the approval gate (agent traffic).
//! See `docs/superpowers/specs/2026-05-15-shell-allowlist-approval-design.md` § 4–6.1.

use std::sync::Arc;

use serde::Serialize;
use schemars::JsonSchema;

use crate::config::ShellConfig;
use crate::exec::host::parse_argv;
use crate::functions::types::ClassifyArgvRequest;

/// Internal outcome before JSON tagging for the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifyOutcome {
    Auto,
    Deny { reason: String },
    Ask { summary: String },
}

/// Serialize as `{ "decision": "auto" | "deny" | "ask", ... }`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "lowercase")]
pub enum ClassifyWireResponse {
    Auto,
    Deny { reason: String },
    Ask { summary: String },
}

impl From<ClassifyOutcome> for ClassifyWireResponse {
    fn from(o: ClassifyOutcome) -> Self {
        match o {
            ClassifyOutcome::Auto => ClassifyWireResponse::Auto,
            ClassifyOutcome::Deny { reason } => ClassifyWireResponse::Deny { reason },
            ClassifyOutcome::Ask { summary } => ClassifyWireResponse::Ask { summary },
        }
    }
}

pub(crate) const SUMMARY_MAX: usize = 512;

pub fn summarize_argv(argv: &[String]) -> String {
    let joined = argv.join(" ");
    if joined.len() <= SUMMARY_MAX {
        joined
    } else {
        let mut s = joined.chars().take(SUMMARY_MAX).collect::<String>();
        s.push_str("… (truncated)");
        s
    }
}

/// Pure classifier for the agent path: denylist → allow_any → allowlist → ask.
/// Empty allowlist does **not** auto-approve here (unlike [`ShellConfig::is_command_allowed`]).
pub fn classify_agent_path(cfg: &ShellConfig, argv: &[String]) -> ClassifyOutcome {
    if argv.is_empty() {
        return ClassifyOutcome::Deny {
            reason: "empty command".into(),
        };
    }
    if let Some(reason) = cfg.denylist_hit_reason(argv) {
        return ClassifyOutcome::Deny { reason };
    }
    if cfg.allow_any {
        return ClassifyOutcome::Auto;
    }
    if cfg.allowlist_contains(argv) {
        return ClassifyOutcome::Auto;
    }
    ClassifyOutcome::Ask {
        summary: summarize_argv(argv),
    }
}

pub async fn handle(
    cfg: Arc<ShellConfig>,
    req: ClassifyArgvRequest,
) -> Result<ClassifyWireResponse, String> {
    let argv = parse_argv(&req.command, req.args.as_ref())?;
    Ok(classify_agent_path(cfg.as_ref(), &argv).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShellConfig;

    fn cfg_allow_deny(allow: Vec<&str>, deny: Vec<&str>) -> ShellConfig {
        let mut c = ShellConfig {
            allowlist: allow.into_iter().map(String::from).collect(),
            denylist_patterns: deny.into_iter().map(String::from).collect(),
            ..Default::default()
        };
        c.compile_denylist().unwrap();
        c
    }

    #[test]
    fn empty_allowlist_empty_denylist_asks() {
        let c = cfg_allow_deny(vec![], vec![]);
        let out = classify_agent_path(&c, &["anything".into()]);
        assert!(matches!(out, ClassifyOutcome::Ask { .. }));
    }

    #[test]
    fn denylist_wins_on_empty_allowlist() {
        let c = cfg_allow_deny(vec![], vec![r"rm\s+-rf\s+/"]);
        match classify_agent_path(&c, &["rm".into(), "-rf".into(), "/".into()]) {
            ClassifyOutcome::Deny { reason } => assert!(reason.contains("denylist")),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn allowlisted_auto() {
        let c = cfg_allow_deny(vec!["ls", "cat"], vec![]);
        assert!(matches!(
            classify_agent_path(&c, &["ls".into(), "-la".into()]),
            ClassifyOutcome::Auto
        ));
    }

    #[test]
    fn allowlist_miss_asks_with_summary() {
        let c = cfg_allow_deny(vec!["ls"], vec![]);
        match classify_agent_path(&c, &["netstat".into(), "-an".into()]) {
            ClassifyOutcome::Ask { summary } => assert_eq!(summary, "netstat -an"),
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn denylist_wins_over_allowlist() {
        let c = cfg_allow_deny(vec!["rm"], vec![r"rm\s+-rf\s+/"]);
        match classify_agent_path(&c, &["rm".into(), "-rf".into(), "/".into()]) {
            ClassifyOutcome::Deny { .. } => {}
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn basename_allowlisted() {
        let c = cfg_allow_deny(vec!["ls"], vec![]);
        assert!(matches!(
            classify_agent_path(&c, &["/usr/bin/ls".into(), "-la".into()]),
            ClassifyOutcome::Auto
        ));
    }

    #[test]
    fn summary_truncates_long_argv() {
        let c = cfg_allow_deny(vec![], vec![]);
        let arg = "a".repeat(300);
        let argv = vec!["cmd".into(), arg.clone(), arg.clone()];
        match classify_agent_path(&c, &argv) {
            ClassifyOutcome::Ask { summary } => {
                assert!(summary.ends_with("… (truncated)"));
                assert!(summary.len() <= SUMMARY_MAX + "… (truncated)".len());
            }
            other => panic!("expected Ask, got {other:?}"),
        }
    }

    #[test]
    fn allow_any_skips_allowlist_when_not_denylisted() {
        let mut c = cfg_allow_deny(vec![], vec![]);
        c.allow_any = true;
        assert!(matches!(
            classify_agent_path(&c, &["netstat".into()]),
            ClassifyOutcome::Auto
        ));
    }

    #[test]
    fn allow_any_still_loses_to_denylist() {
        let mut c = cfg_allow_deny(vec![], vec![r"bad"]);
        c.allow_any = true;
        assert!(matches!(
            classify_agent_path(&c, &["badcmd".into()]),
            ClassifyOutcome::Deny { .. }
        ));
    }
}
