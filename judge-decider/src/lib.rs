//! decider provider for the judge hub: decider-4b (a Qwen3.5-4B-Base fine-tune
//! for typed decisions) run in-process through llama.cpp, reading the
//! next-token logits of the option labels (no decoding).
mod cancellation;
pub mod client;
pub mod config;
pub mod configuration;
pub mod download;
pub mod engine;
pub mod prompt;
pub mod register;
pub use client::{DeciderClient, Limits};
pub use config::DeciderConfig;
pub use configuration::SharedConfig;
pub use register::register;
#[cfg(feature = "console-ui")]
pub mod ui;

/// Suffix the `judge` hub selects this worker by (`judge-decider`).
pub const PROVIDER: &str = "decider";
pub const EVALUATE_ID: &str = "judge-decider::evaluate";
pub const MODELS_ID: &str = "judge-decider::models::list";
pub const CANCEL_ID: &str = "judge-decider::cancel";
