//! `browser::snapshot` — accessibility-tree outline with element refs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SnapshotInput {
    pub session_id: String,
    /// Return only what changed since this session's previous snapshot
    /// instead of the full outline. Falls back to a full snapshot when
    /// there is no baseline (first snapshot, or first after a navigation).
    #[serde(default)]
    pub diff: Option<bool>,
}

/// Changes since the previous snapshot. Lines are compared without their
/// `[ref=eN]` suffix (ref names are unique per snapshot); `added` lines
/// carry current refs and are directly actionable.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SnapshotDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SnapshotOutput {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Indented outline; lines carry `[ref=eN]` handles for `browser::act`.
    /// Empty when `diff` is populated.
    pub tree: String,
    /// True when the tree hit `max_snapshot_nodes` and was cut short.
    pub truncated: bool,
    /// Document generation the refs belong to; navigation advances it and
    /// kills every ref from earlier generations.
    pub generation: u64,
    /// Present when the caller asked for `diff: true` and a baseline existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<SnapshotDiff>,
}
