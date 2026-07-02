/// Trigger type registered when `III_HTTP_TRIGGER_TYPE` is unset. Safe to run
/// alongside the built-in `iii-http` worker, which owns `http`.
pub const DEFAULT_TRIGGER_TYPE: &str = "http-ng";

/// The trigger type this worker registers. Reads `III_HTTP_TRIGGER_TYPE`
/// (default `http-ng`, which is safe to run alongside the built-in iii-http).
/// Set it to `http` for the drop-in cutover once iii-http is removed.
pub fn trigger_type() -> String {
    std::env::var("III_HTTP_TRIGGER_TYPE").unwrap_or_else(|_| DEFAULT_TRIGGER_TYPE.to_string())
}

pub mod boot;
pub mod condition;
pub mod config;
pub mod configuration;
pub mod handler;
pub mod manifest;
pub mod middleware;
pub mod server;
pub mod trigger;
pub mod types;
