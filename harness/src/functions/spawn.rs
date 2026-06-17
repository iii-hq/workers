//! `harness::spawn` — spawn a sub-agent in a child session (harness.md §
//! Sub-agents). Designed to be called by the model through `agent_trigger`;
//! the dispatch layer records the child linkage and reports the call pending.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::MessageInput;
use crate::types::model::ThinkingLevel;
use crate::types::output::OutputContract;
use crate::types::turn::FunctionPolicy;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SpawnOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Capped at the parent's remaining turn budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// The child's deliverable: text / json / json+schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputContract>,
    /// Intersected with the parent policy — narrow, never escalate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// Fan-out guard for the child's own spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_children: Option<u32>,
    /// Parent-side wait guard for this child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnRequest {
    /// The child's goal — its opening user message.
    pub task: MessageInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Spawn into an existing session (e.g. a fork); default: create fresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SpawnOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnResponse {
    pub child_session_id: String,
    pub child_turn_id: String,
}

/// Direct-call entry (a consumer starting a linked child). Dispatched from a
/// turn, the dispatch layer handles linkage + pending; here we start a child
/// and return its ids. Parent linkage is injected by the dispatcher, never
/// trusted from model arguments — a direct call has no parent.
pub async fn handle(deps: &Deps, req: SpawnRequest) -> Result<SpawnResponse, HarnessError> {
    let ids = crate::subagent::spawn_child(deps, &req, None).await?;
    Ok(SpawnResponse {
        child_session_id: ids.session_id,
        child_turn_id: ids.turn_id,
    })
}
