//! `budget::*` — workspace + agent LLM spend caps with alerts, forecast, and
//! period rollover. Graduated from roster/workers/llm-budget (TS) in P4.
//!
//! See [`register_with_iii`] for the full list of registered functions.

pub mod ops;
pub mod periods;
pub mod register;
pub mod store;

pub use periods::{next_period_start, period_key, period_start, Period};
pub use register::{register_with_iii, BudgetFunctionRefs};
pub use store::{Alert, Budget, Exemption, SpendLogEntry};
