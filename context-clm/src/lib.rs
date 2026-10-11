//! `context-clm`: a harness context strategy where the model manages its own
//! context. Everything after the task is mirrored to a file inside the
//! session's filesystem root; the model edits that file with ordinary tools
//! and the edited file becomes its context on the next request, under a
//! token budget enforced by a controller.

/// Worker identity: catalog slug, binary and registered worker name.
pub const WORKER_NAME: &str = "context-clm";

pub mod assemble;
pub mod config;
pub mod configuration;
pub mod controller;
pub mod mirror;
pub mod projection;
pub mod prompt;
pub mod store;
pub mod types;
