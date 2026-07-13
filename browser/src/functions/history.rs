//! `browser::history` — back / forward / reload on the session's page.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryInput {
    pub session_id: String,
    /// `back`, `forward`, or `reload`.
    pub action: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HistoryOutput {
    pub ok: bool,
    /// URL after the action. `back`/`forward` at the history edge is a
    /// no-op with ok=true.
    pub url: String,
    /// False when back/forward had no entry to move to.
    pub moved: bool,
}
