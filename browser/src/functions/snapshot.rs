//! `browser::snapshot` — accessibility-tree outline with element refs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SnapshotOutput {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Indented outline; lines carry `[ref=eN]` handles for `browser::act`.
    pub tree: String,
    /// True when the tree hit `max_snapshot_nodes` and was cut short.
    pub truncated: bool,
}
