//! `rbac-proxy::status` — health/identity probe for `iii worker info` smoke
//! tests and dashboards. Read-only operational metadata; sync invocation.
//!
//! The `engine_url` is passed through [`crate::redact_url`] before it leaves
//! the worker, so a credentialed `wss://user:secret@host` upstream never leaks
//! to an agent-callable probe.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const STATUS_ID: &str = "rbac-proxy::status";
pub const STATUS_DESC: &str =
    "Health/identity probe: bound host/port, the (credential-redacted) upstream engine URL, whether an auth function is configured, and the live downstream connection count.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct StatusOutput {
    /// Bound public host.
    pub host: String,
    /// Bound public RBAC port.
    pub port: u16,
    /// Upstream engine URL, with any `user:pass@` credentials redacted.
    pub engine_url: String,
    /// Whether an `auth_function_id` is configured.
    pub rbac_enabled: bool,
    /// Live downstream connections.
    pub active_connections: u32,
    /// Worker version (matches `Cargo.toml`).
    pub version: String,
}
