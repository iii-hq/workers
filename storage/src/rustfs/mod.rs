//! Lifecycle for the optional rustfs sidecar (used by `provider: local`).

pub mod health;
pub mod spawn;

pub use spawn::{discover_binary, ephemeral_credentials, RustfsHandle, RustfsSpawnOpts};
