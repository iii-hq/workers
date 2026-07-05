//! `harness::spawn` — spawn a sub-agent in a child session (harness.md §
//! Sub-agents). Designed to be called by the model through `agent_trigger`;
//! the dispatch layer records the child linkage and reports the call pending.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::MessageInput;
use crate::prompt::{Mode, SystemPromptStrategy};
use crate::types::model::ThinkingLevel;
use crate::types::output::OutputContract;
use crate::types::turn::FunctionPolicy;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SpawnOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// How `system_prompt` combines with the built-in prompt: `override`
    /// replaces it; `enrich` (default) appends to it.
    #[serde(default)]
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
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
    /// Workspace isolation for the child. `worktree` gives it its own git
    /// worktree (`.worktrees/<name>` on branch `wt/<name>` under the
    /// parent's filesystem root) so parallel children never edit the same
    /// tree; the parent merges `wt/<name>` when the child finishes.
    /// Requires the parent turn to have a filesystem root inside a git
    /// repository. Dispatch-path spawns only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<Isolation>,
}

/// Child workspace isolation modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    Worktree,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnRequest {
    /// The child's goal — its opening user message.
    pub task: MessageInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Spawn into this session, creating it if it does not exist (e.g. a fork,
    /// or a pre-chosen id to filter `turn-completed` subscriptions on); default:
    /// create fresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Display-only parent for the console session tree, used when there is no
    /// live parent turn (e.g. a trigger-fired spawn from `harness::react`).
    /// Writes `SessionMeta.metadata.parent_session_id` so the console nests this
    /// child; it does NOT grant policy inheritance or parent-call resolution.
    /// Ignored when the dispatcher injects a real parent link (an in-turn spawn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Stamped by `harness::react` (not caller-supplied): the subscription that
    /// spawned this turn. Its completion event is never delivered back to that
    /// same subscription (self-edge loop breaker).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by_subscription_id: Option<String>,
    /// Stamped by `harness::react` (not caller-supplied): reactive-chain depth,
    /// echoed on this turn's `turn-completed` event so react can cap runaway
    /// chains at `MAX_REACTIVE_DEPTH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactive_depth: Option<u32>,
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
