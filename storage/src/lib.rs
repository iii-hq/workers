//! storage worker — public surface for the binary and tests.

pub mod backend;
pub mod config;
pub mod configuration;
pub mod error;
pub mod handlers;
pub mod manifest;
pub mod triggers;
pub mod ui;

pub fn worker_name() -> &'static str {
    "storage"
}
