//! clef provider for the judge hub: Cloudflare's Clef-Flash decision model run
//! in-process. llama.cpp runs the Qwen3.5 backbone for the final hidden state
//! of every prompt token; the joint schema head (candle, CPU) turns them into
//! one logit per option of every question, all questions in one pass.
mod cancellation;
pub mod client;
pub mod config;
pub mod configuration;
pub mod download;
pub mod encode;
pub mod engine;
pub mod head;
pub mod register;
pub use client::{ClefClient, Limits};
pub use config::ClefConfig;
pub use configuration::SharedConfig;
pub use register::register;
#[cfg(feature = "console-ui")]
pub mod ui;

/// Suffix the `judge` hub selects this worker by (`judge-clef`).
pub const PROVIDER: &str = "clef";
pub const EVALUATE_ID: &str = "judge-clef::evaluate";
pub const MODELS_ID: &str = "judge-clef::models::list";
pub const CANCEL_ID: &str = "judge-clef::cancel";
