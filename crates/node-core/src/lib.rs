//! iii-node-core: evaluate untrusted JavaScript in per-namespace V8 isolates.
//!
//! Everything here is bus-free. The only way this crate reaches an iii engine
//! is through [`engine::Engine`], a five-method trait over `serde_json::Value`
//! that the hosting worker implements against its own connection — so nothing
//! in this tree depends on `iii-sdk`, and nothing in it pins an SDK version.
//!
//! The wire types (`RunRequest`, `RunResponse`, …), the function registrations
//! and the console UI all live with that worker.

pub mod allocator;
pub mod config;
pub mod engine;
pub mod error;
pub mod ids;
pub mod manager;
pub mod ops;
pub mod protocol;
pub mod runtime;
pub mod wire;
