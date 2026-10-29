//! database worker — public surface for the binary and tests.

pub mod config;
pub mod configuration;
pub mod connector;
pub(crate) mod driver;
pub mod error;
pub mod handle;
pub mod handlers;
pub mod pool;
pub mod redaction;
pub mod runtime;
pub mod secrets;
pub mod ssh;
pub mod transaction;
pub mod triggers;
pub mod ui;
pub mod value;

pub fn worker_name() -> &'static str {
    "database"
}
