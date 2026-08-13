//! The request/response types for the three operations `RuntimeManager`
//! exposes.
//!
//! They live here rather than with the hosting worker because
//! `RuntimeManager`'s own methods take and return them. Nothing in this module
//! touches the bus — `serde` and `schemars` derives only — so the crate stays
//! SDK-free; the worker re-exports them under its `functions` module, which is
//! what registers them.

pub mod register;
pub mod run;
pub mod teardown;
