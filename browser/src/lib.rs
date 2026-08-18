//! Library surface for the `browser` worker: interactive Chromium sessions
//! on the iii bus, plus the native `browser::*` HTML-parsing
//! surface. The binary (`src/main.rs`) is a thin boot sequence; everything
//! testable lives here.

pub mod config;
pub mod configuration;
pub mod events;
pub mod functions;
pub mod logging;
pub mod manifest;
pub mod scrapling;
pub mod session;
pub mod snapshot;
pub mod ssrf;
pub mod ui;
