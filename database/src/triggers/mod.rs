//! Trigger background tasks. Each trigger runs as its own tokio task spawned
//! at worker startup.
//!
//! Only `handler` is part of the public crate surface (consumed by main.rs).
//! `query_poll` and `row_change` are implementation modules.

pub mod handler;
pub(crate) mod query_poll;
pub(crate) mod row_change;
