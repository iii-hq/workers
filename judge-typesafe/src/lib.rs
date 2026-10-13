//! TypeSafe JEV provider for the `judge` hub; the bus contract is shared through `judge_contract`.
/// TypeSafe model used when neither configuration nor the request names one.
pub const DEFAULT_MODEL: &str = "jev-latest";
/// Suffix the `judge` hub selects this worker by (`judge-typesafe`).
pub const PROVIDER: &str = "typesafe";
pub const EVALUATE_ID: &str = "judge-typesafe::evaluate";
pub const MODELS_ID: &str = "judge-typesafe::models::list";
pub const CANCEL_ID: &str = "judge-typesafe::cancel";
/// Evicts a cached `secret://` key when the `secrets` worker changes it.
pub const SECRET_CHANGED_ID: &str = "judge-typesafe::on-secret-change";
pub mod client;
pub use client::JevClient;
pub use judge_provider::{ExecutionLimits, RetryPolicy, DEFAULT_RETRY};
pub mod config;
pub mod configuration;
pub mod register;
pub use config::JevConfig;
pub use configuration::SharedConfig;
pub use register::register;
#[cfg(feature = "console-ui")]
pub mod ui;
