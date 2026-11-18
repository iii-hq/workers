//! Provider-neutral judge hub: `judge::*` forwarded to `judge-<provider>::*`.
pub mod config;
pub mod configuration;
pub mod mentions;
pub mod register;
pub use config::JudgeConfig;
pub use configuration::SharedConfig;
pub use register::{DEFAULT_PROVIDER, register, validate_provider};
#[cfg(feature = "console-ui")]
pub mod ui;
