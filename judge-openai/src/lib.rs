//! OpenAI Decisions provider for the `judge` hub; the bus contract is shared through `judge_contract`.
/// OpenAI model used when neither configuration nor the request names one.
pub const DEFAULT_MODEL: &str = "gpt-6-luna";
/// The only models the Decisions API serves; anything else is `invalid_request`
/// before any HTTP, and `models::list` reports only these.
pub const SUPPORTED_MODELS: &[&str] = &["gpt-6-luna"];
/// Suffix the `judge` hub selects this worker by (`judge-openai`).
pub const PROVIDER: &str = "openai";
pub const EVALUATE_ID: &str = "judge-openai::evaluate";
pub const MODELS_ID: &str = "judge-openai::models::list";
pub const CANCEL_ID: &str = "judge-openai::cancel";
/// Evicts a cached `secret://` key when the `secrets` worker changes it.
pub const SECRET_CHANGED_ID: &str = "judge-openai::on-secret-change";
pub mod client;
pub use client::DecisionsClient;
pub use judge_provider::{ExecutionLimits, RetryPolicy, DEFAULT_RETRY};
pub mod config;
pub mod configuration;
mod decisions;
pub mod register;
pub use config::OpenAiConfig;
pub use configuration::SharedConfig;
pub use register::register;
#[cfg(feature = "console-ui")]
pub mod ui;
