//! Step-definition modules. One per feature group under
//! `tests/features/`. Most steps live in `common` (engine readiness,
//! generic dispatcher, fixture writes, shared outcome assertions); the
//! per-function modules add response-shape assertions specific to each
//! `coder::*` function.

pub mod common;
pub mod create;
pub mod delete;
pub mod lifecycle;
pub mod list;
pub mod read;
pub mod search;
pub mod security;
pub mod tree;
pub mod update;
