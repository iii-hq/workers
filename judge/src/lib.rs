//! Provider-neutral judge hub: `judge::*` forwarded to `judge-<provider>::*`.
pub mod manifest;
pub mod register;
pub use register::{register, validate_provider, DEFAULT_PROVIDER};
