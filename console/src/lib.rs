//! `console` worker library surface — exposed for integration tests and
//! sibling crates that want to embed parts of the worker.

pub mod assets;
pub mod config;
pub mod configuration;
pub mod functions;
pub mod manifest;
pub mod proxy;
pub mod server;
pub mod ui;
pub mod ui_assets;

pub fn worker_name() -> &'static str {
    "console"
}
