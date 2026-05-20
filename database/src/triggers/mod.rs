//! Trigger background tasks. Each trigger runs as its own tokio task spawned
//! at worker startup.
//!
//! Only `handler` is part of the public crate surface (consumed by main.rs).
//! `row_change` is an implementation module.

pub mod handler;
pub(crate) mod row_change;
