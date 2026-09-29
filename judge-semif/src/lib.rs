//! SemIf provider for the judge hub: a frozen GGUF LLM run in-process through
//! llama.cpp, reading the next-token logits of the option letters (no decoding).
mod cancellation;
pub mod client;
pub mod config;
pub mod configuration;
pub mod download;
pub mod engine;
pub mod prompt;
pub mod register;
pub use client::{Limits, SemifClient};
pub use config::SemifConfig;
pub use configuration::SharedConfig;
pub use register::register;
#[cfg(feature = "console-ui")]
pub mod ui;

/// Suffix the `judge` hub selects this worker by (`judge-semif`).
pub const PROVIDER: &str = "semif";
pub const EVALUATE_ID: &str = "judge-semif::evaluate";
pub const MODELS_ID: &str = "judge-semif::models::list";
pub const CANCEL_ID: &str = "judge-semif::cancel";
