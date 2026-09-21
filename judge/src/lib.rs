//! Provider-neutral judge hub: `judge::*` forwarded to `judge-<provider>::*`.
pub mod config;
pub mod configuration;
pub mod manifest;
pub mod register;
pub use config::JudgeConfig;
pub use configuration::SharedConfig;
pub use register::{register, validate_provider, DEFAULT_PROVIDER};
#[cfg(feature = "console-ui")]
pub mod ui;
