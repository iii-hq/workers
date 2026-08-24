pub mod catalog;
pub mod config;
pub mod discovery;
pub mod errors;
pub mod manifest;
pub mod register;
pub mod request;
pub mod sse;
pub mod stream_fn;
pub mod surface;
pub mod upstream;
pub mod wire;

pub const PROVIDER_ID: &str = "command-code";
pub const STATE_SCOPE: &str = "provider-command-code";

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
