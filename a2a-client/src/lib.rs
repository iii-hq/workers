//! Library surface of `iii-a2a-client` so integration tests (and any
//! downstream embedder) can reuse the module pieces. The binary in
//! `src/main.rs` consumes these via `crate::*`; this `lib.rs` re-exposes
//! them under the package name.

pub mod registration;
pub mod session;
pub mod task;
pub mod types;
