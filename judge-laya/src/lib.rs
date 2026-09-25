//! laya provider for the judge hub: ModernBERT encoder from candle plus laya's
//! typed decision head, downloaded from the Hugging Face Hub and run in-process.
mod cancellation;
pub mod client;
pub mod config;
pub mod configuration;
pub mod download;
pub mod encode;
pub mod engine;
pub mod gguf;
pub mod lang;
pub mod model;
pub mod register;
pub use client::{LayaClient, Limits, Routing};
pub use config::LayaConfig;
pub use configuration::SharedConfig;
pub use register::register;
#[cfg(feature = "console-ui")]
pub mod ui;

/// Suffix the `judge` hub selects this worker by (`judge-laya`).
pub const PROVIDER: &str = "laya";
pub const EVALUATE_ID: &str = "judge-laya::evaluate";
pub const MODELS_ID: &str = "judge-laya::models::list";
pub const CANCEL_ID: &str = "judge-laya::cancel";
