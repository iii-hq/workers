//! Canonical artifact persistence and run-relative path registration.

mod sink;

#[cfg(test)]
mod tests;

pub use sink::{write_json, ArtifactSink};

#[cfg(test)]
pub(crate) use sink::trim_passing_run;
