//! storage worker — public surface for the binary and tests.

pub mod backend;
pub mod config;
pub mod error;
pub mod handlers;
pub mod rustfs;
pub mod triggers;

pub fn worker_name() -> &'static str {
    "storage"
}
