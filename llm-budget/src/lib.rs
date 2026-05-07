//! `budget::*` — workspace + agent LLM spend caps with alerts, forecast, and
//! period rollover. Graduated from roster/workers/llm-budget (TS) in P4.
//!
//! See [`register_with_iii`] for the full list of registered functions.

pub const SKILL_ID: &str = "llm-budget";
pub const SKILL_MD: &str = include_str!("../skill.md");

pub mod config;
pub mod manifest;

pub const SUB_SKILLS: &[(&str, &str)] = &[
    ("llm-budget/create", include_str!("../skills/create.md")),
    ("llm-budget/list", include_str!("../skills/list.md")),
    ("llm-budget/get", include_str!("../skills/get.md")),
    ("llm-budget/update", include_str!("../skills/update.md")),
    ("llm-budget/delete", include_str!("../skills/delete.md")),
    ("llm-budget/reset", include_str!("../skills/reset.md")),
    ("llm-budget/check", include_str!("../skills/check.md")),
    ("llm-budget/record", include_str!("../skills/record.md")),
    ("llm-budget/usage", include_str!("../skills/usage.md")),
    ("llm-budget/forecast", include_str!("../skills/forecast.md")),
    (
        "llm-budget/alert_set",
        include_str!("../skills/alert_set.md"),
    ),
    ("llm-budget/enforce", include_str!("../skills/enforce.md")),
    ("llm-budget/exempt", include_str!("../skills/exempt.md")),
    ("llm-budget/pause", include_str!("../skills/pause.md")),
];

pub mod ops;
pub mod periods;
pub mod register;
pub mod store;

pub use periods::{next_period_start, period_key, period_start, Period};
pub use register::{register_with_iii, BudgetFunctionRefs};
pub use store::{Alert, Budget, Exemption, SpendLogEntry};
