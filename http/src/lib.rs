/// The trigger type this worker registers. Always `http` -- the built-in
/// `iii-http` worker must be removed from the engine config before this
/// worker can boot (see [`boot::start`]'s guard).
pub const TRIGGER_TYPE: &str = "http";

pub mod boot;
pub mod condition;
pub mod config;
pub mod configuration;
pub mod handler;
pub mod logging;
pub mod manifest;
pub mod middleware;
pub mod observability;
pub mod server;
pub mod trigger;
pub mod types;
