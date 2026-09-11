//! `console::status` — health/identity probe.
//!
//! Returns the runtime knobs the worker is operating under so callers
//! (operators, `iii worker info` smoke tests, dashboards) can confirm
//! the worker is alive and tell which port the UI is on without parsing
//! logs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusOutput {
    /// TCP port the worker is serving the UI and `/ws` on.
    pub http_port: u16,
    /// iii engine WebSocket URL the worker is proxying to.
    pub engine_url: String,
    /// Worker version (matches `Cargo.toml`).
    pub version: String,
}
