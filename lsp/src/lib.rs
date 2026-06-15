//! Library surface for `lsp`.
//!
//! The binary (`src/main.rs`) keeps its own `mod` declarations; this lib
//! target re-exports the same modules so integration tests under `tests/`
//! can exercise the engine-introspection contract without spawning a
//! language-server process.

pub mod analyzer;
pub mod completions;
pub mod diagnostics;
pub mod engine_client;
pub mod engine_introspection;
pub mod hover;
