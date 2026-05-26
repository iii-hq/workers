//! Library facade exposing the binary's modules so integration tests
//! under `tests/` can drive them at the public-API level. Both targets
//! share source files via Cargo's two-target compile.

pub mod config;
pub mod error;
pub mod functions;
pub mod manifest;
pub mod path;
