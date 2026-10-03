//! clef provider for the judge hub: Cloudflare's Clef-Flash decision model run
//! in-process. llama.cpp runs the Qwen3.5 backbone for the final hidden state
//! of every prompt token; the joint schema head (candle, CPU) turns them into
//! one logit per option of every question, all questions in one pass.
// ponytail: the `legacy` gates go in stage 2 (Cargo.toml).
#[cfg(feature = "legacy")]
mod cancellation;
#[cfg(feature = "legacy")]
pub mod client;
#[cfg(feature = "legacy")]
pub mod config;
#[cfg(feature = "legacy")]
pub mod configuration;
pub mod download;
pub mod encode;
#[cfg(feature = "legacy")]
pub mod engine;
#[cfg(feature = "legacy")]
pub mod head;
#[cfg(not(feature = "legacy"))]
pub mod llama;
#[cfg(feature = "legacy")]
pub mod register;
#[cfg(feature = "legacy")]
pub use client::{ClefClient, Limits};
#[cfg(feature = "legacy")]
pub use config::ClefConfig;
#[cfg(feature = "legacy")]
pub use configuration::SharedConfig;
#[cfg(feature = "legacy")]
pub use register::register;
#[cfg(all(feature = "console-ui", feature = "legacy"))]
pub mod ui;

/// Suffix the `judge` hub selects this worker by (`judge-clef`).
pub const PROVIDER: &str = "clef";
pub const EVALUATE_ID: &str = "judge-clef::evaluate";
pub const MODELS_ID: &str = "judge-clef::models::list";
pub const CANCEL_ID: &str = "judge-clef::cancel";
