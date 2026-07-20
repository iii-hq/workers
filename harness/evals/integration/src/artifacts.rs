//! Canonical artifact persistence and run-relative path registration.

mod sink;

#[cfg(test)]
mod tests;

pub use sink::{trim_passing_run, write_json, ArtifactSink};
