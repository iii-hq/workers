//! iii-database worker — public surface for the binary and tests.
//!
//! `pub` modules are the consumed surface (main.rs + integration tests).
//! `pub(crate)` modules are internal — keeping them tight prevents callers
//! from coupling to types that may move/rename without notice.

pub mod config;
pub(crate) mod cursor;
pub(crate) mod driver;
pub mod error;
pub mod handle;
pub mod handlers;
pub mod pool;
pub mod triggers;
pub mod value;

pub fn worker_name() -> &'static str {
    "iii-database"
}
