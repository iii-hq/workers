//! Per-generation context snapshot (`harness_context/<session_id>`): what
//! the last generation's model window held, by category, plus the provider
//! usage that came back for it. Built from the assembly the loop already
//! performs (no extra counting round trips), stamped with usage after the
//! terminal frame, stored once per generate step. Read back by
//! `harness::metrics` and pushed on `harness::turn-completed`.

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::HarnessError;
use crate::types::event::Usage;

pub const CONTEXT_SCOPE: &str = "harness_context";

/// Estimated tokens of the assembled window's messages, by role.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotMessagesV1 {
    pub user: u64,
    pub assistant: u64,
    pub function_result: u64,
    pub custom: u64,
}

/// Where the request's tokens sit. Categories are assembly-time estimates;
/// `hook_guidance` is the measured growth after assembly (pre-generate hook
/// appends and orphan-repair patches), 0 when the request left assembly
/// unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotCategoriesV1 {
    /// Final assembled system prompt: mode paragraph, identity, per-step
    /// aids, and any compaction summary section.
    pub system_prompt: u64,
    /// Function schemas exposed to the model.
    pub tools: u64,
    pub messages: SnapshotMessagesV1,
    /// Provider framing plus response_format / provider_options fields.
    pub overhead: u64,
    #[serde(default)]
    pub hook_guidance: u64,
}

/// One generation's context accounting. `total <= usable` always holds for
/// a generation that ran; `free = usable - total`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextSnapshotV1 {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Which estimator produced the numbers (`heuristic` until the
    /// context-manager resolves a real tokenizer). Absent when the
    /// context-manager predates the breakdown response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator: Option<String>,
    /// The input budget the window was fit into.
    pub usable: u64,
    /// Output allocation `usable` was derived against.
    pub effective_max_output_tokens: u64,
    /// Final request estimate: categories plus post-assembly growth.
    pub total: u64,
    pub free: u64,
    pub categories: SnapshotCategoriesV1,
    pub compacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarized_head_tokens: Option<u64>,
    /// Actual provider usage for this generation, stamped after the
    /// terminal frame; absent when the provider returned none (or the
    /// generation never completed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub timestamp: i64,
}

/// Store the session's latest snapshot (whole-value write; the loop holds
/// the only writer per session).
pub async fn put(
    iii: &IIIClient,
    snapshot: &ContextSnapshotV1,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(snapshot)
        .map_err(|e| HarnessError::State(format!("context snapshot serialize: {e}")))?;
    crate::state::state_set(iii, CONTEXT_SCOPE, &snapshot.session_id, value, timeout_ms).await
}

/// The session's latest snapshot (`None` when absent or unparseable — a
/// snapshot from a newer harness must degrade to "no data", never an error).
pub async fn get(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Option<ContextSnapshotV1>, HarnessError> {
    let v = crate::state::state_get(iii, CONTEXT_SCOPE, session_id, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(v).ok())
}

pub async fn delete(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    crate::state::state_delete(iii, CONTEXT_SCOPE, session_id, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_and_tolerates_unknown_fields() {
        let snapshot = ContextSnapshotV1 {
            session_id: "s_1".into(),
            turn_id: "t_1".into(),
            step: 3,
            model: "m".into(),
            provider: Some("p".into()),
            estimator: Some("heuristic".into()),
            usable: 100_000,
            effective_max_output_tokens: 8_192,
            total: 62_000,
            free: 38_000,
            categories: SnapshotCategoriesV1 {
                system_prompt: 2_400,
                tools: 9_800,
                messages: SnapshotMessagesV1 {
                    user: 10_000,
                    assistant: 20_000,
                    function_result: 19_000,
                    custom: 0,
                },
                overhead: 300,
                hook_guidance: 500,
            },
            compacted: true,
            summarized_head_tokens: Some(3_100),
            usage: Some(Usage {
                input: Some(61_400),
                output: Some(900),
                cache_read: Some(55_000),
                cache_write: None,
                reasoning: None,
                cost_usd: Some(0.42),
            }),
            timestamp: 1_722_700_000_000,
        };
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["from_the_future"] = serde_json::json!(true);
        let parsed: ContextSnapshotV1 = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, snapshot);
    }
}
