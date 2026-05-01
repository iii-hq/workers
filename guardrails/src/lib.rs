//! `guardrails::*` — local heuristics for PII, leaked API keys, jailbreak
//! keywords, and toxicity. Graduated from roster/workers/guardrails (TS) in P4.
//!
//! Functions registered:
//! - `guardrails::check_input`  — `{ text, rules? } → { allowed, reasons, redacted? }`
//! - `guardrails::check_output` — same shape; same ruleset (leaked keys + PII
//!   are first-class on the output lane).
//! - `guardrails::classify`     — `{ text } → { pii, jailbreak, toxicity, keys_leaked }`

pub mod register;
pub mod rules;
pub mod scan;

pub use register::{register_with_iii, GuardrailsFunctionRefs};
pub use rules::{Category, Pattern};
pub use scan::{run_checks, CheckResult, Rules};
