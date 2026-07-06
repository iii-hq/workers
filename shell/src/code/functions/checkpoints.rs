//! `coder::checkpoints` — bounded listing of the workspace write journal
//! (newest first) so a caller can pick an undo target: by recency
//! (`coder::undo { steps }`) or by turn (`coder::undo { turn_id }`).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::code::journal;
use crate::code::path::PathResolver;

/// Default listing cap.
const DEFAULT_LIMIT: u32 = 50;

// examples are wire-contract; goldens pin them.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[schemars(example = "example_checkpoints_input")]
pub struct CheckpointsInput {
    /// Maximum records returned (newest first). Default 50.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_checkpoints_input() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CheckpointsOutput {
    /// Journal records, newest first.
    pub records: Vec<CheckpointMeta>,
    /// True when older records exist beyond `limit`.
    pub truncated: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CheckpointMeta {
    pub seq: u64,
    /// Unix millis when the mutation was journaled.
    pub ts: i64,
    /// The mutation (e.g. "coder::apply-patch", "coder::undo").
    pub function_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Affected file paths.
    pub files: Vec<String>,
    /// Count of unrecoverable entries in this record (oversized images,
    /// directory operations).
    pub skipped: u32,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: CheckpointsInput,
) -> Result<CheckpointsOutput, String> {
    let scope_root = crate::fs::scope_root(req.fs_scope.as_ref()).map(str::to_string);
    tokio::task::spawn_blocking(move || {
        let root = resolver.effective_root(scope_root.as_deref());
        let mut records = journal::list(&cfg, &root);
        records.reverse(); // newest first
        let limit = req.limit.unwrap_or(DEFAULT_LIMIT).max(1) as usize;
        let truncated = records.len() > limit;
        let records = records
            .into_iter()
            .take(limit)
            .map(|r| CheckpointMeta {
                seq: r.seq,
                ts: r.ts,
                function_id: r.function_id,
                session_id: r.session_id,
                turn_id: r.turn_id,
                files: r.entries.iter().map(|e| e.path.clone()).collect(),
                skipped: r.entries.iter().filter(|e| e.skipped).count() as u32,
            })
            .collect();
        Ok(CheckpointsOutput { records, truncated })
    })
    .await
    .map_err(|e| format!("checkpoints task join failed: {e}"))?
}
