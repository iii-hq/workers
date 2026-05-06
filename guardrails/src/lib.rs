//! `guardrails::*` — local heuristics for PII, leaked API keys, jailbreak
//! keywords, and toxicity. Graduated from roster/workers/guardrails (TS) in P4.
//!
//! Functions registered:
//! - `guardrails::check_input`  — `{ text, rules? } → { allowed, reasons, redacted? }`
//! - `guardrails::check_output` — same shape; same ruleset (leaked keys + PII
//!   are first-class on the output lane).
//! - `guardrails::classify`     — `{ text } → { pii, jailbreak, toxicity, keys_leaked }`

pub const SKILL_ID: &str = "guardrails";
pub const SKILL_MD: &str = include_str!("../skill.md");

pub const SUB_SKILLS: &[(&str, &str)] = &[
    (
        "guardrails/check_input",
        include_str!("../skills/check_input.md"),
    ),
    (
        "guardrails/check_output",
        include_str!("../skills/check_output.md"),
    ),
    ("guardrails/classify", include_str!("../skills/classify.md")),
];

pub mod register;
pub mod rules;
pub mod scan;

pub use register::{register_with_iii, GuardrailsFunctionRefs};
pub use rules::{Category, Pattern};
pub use scan::{run_checks, CheckResult, Rules};
