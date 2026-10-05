//! `secrets`: a local protected store for credentials referenced from
//! versioned configuration as `secret://NAME`.
//!
//! Values are sealed with XChaCha20-Poly1305 in `<data_dir>/vault.json`
//! under a master key that never lives in the project (`III_SECRETS_KEY`, or
//! a key file under `~/.config/iii/secrets`). Only workers named in a
//! secret's `consumers` can resolve it; everything else sees metadata.
pub mod access;
pub mod api;
pub mod config;
pub mod configuration;
pub mod crypto;
pub mod detect;
pub mod error;
pub mod events;
pub mod fsutil;
pub mod functions;
pub mod keys;
pub mod names;
pub mod secret;
pub mod store;
pub mod vault;

pub use error::SecretsError;
pub use functions::{register, Ctx};
pub use secret::SecretString;
pub use store::{Store, StorePaths};

/// The SDK records every handler's input and output payload as span events
/// (`iii.invocation.input` / `iii.invocation.output`), redacting only keys
/// such as `api_key` or `secret`, not `value`. `secrets::set` receives a
/// value and `secrets::resolve` returns one, so this worker always turns
/// payload capture off. Call before any thread is spawned.
pub fn disable_trace_payloads() {
    std::env::set_var("III_DISABLE_TRACE_PAYLOADS", "1");
}
