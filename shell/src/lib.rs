//! Library facade exposing the binary's modules so integration tests
//! under `tests/` can drive them at the public-API level. Both targets
//! share source files via Cargo's two-target compile.

pub mod code;
pub mod config;
pub mod configuration;
pub mod exec;
pub mod exec_dispatch;
pub mod fs;
pub mod functions;
pub mod grant;
pub mod jobs;
pub mod path;
pub mod scode;
pub mod target;
pub mod telemetry;
pub mod triggers;
