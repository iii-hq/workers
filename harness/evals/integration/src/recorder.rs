//! The controlled recorder: run-scoped target functions and the lifecycle
//! sink are registered with the engine, while configuration, reset, and
//! snapshots remain direct runner operations.
//!
//! [`Recorder`] owns the SDK-facing service. The durable event log and its
//! ordered in-memory view are isolated in the private store module. Every
//! accepted target or lifecycle call is written and fsynced before its handler
//! responds.

mod service;
mod store;

pub(crate) use service::const_response_schema;
pub use service::Recorder;

#[cfg(test)]
mod tests;
