//! iii-database worker — public surface for the binary and tests.

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
