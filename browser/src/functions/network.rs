//! `browser::network::read` — filtered read over the network ring buffer.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session::NetworkEntry;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NetworkReadInput {
    pub session_id: String,
    /// Regex applied to the request URL.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Only failed requests (network error or status >= 400).
    #[serde(default)]
    pub failed_only: Option<bool>,
    /// Only entries with `seq` greater than this.
    #[serde(default)]
    pub since_seq: Option<u64>,
    /// Maximum entries returned, newest kept (default 100).
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NetworkReadOutput {
    pub entries: Vec<NetworkEntry>,
    /// Cursor for the next `since_seq`.
    pub last_seq: u64,
    /// Entries evicted from the ring buffer since session start.
    pub dropped: u64,
}
