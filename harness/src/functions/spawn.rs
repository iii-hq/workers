//! `harness::spawn` — spawn a sub-agent in a child session (harness.md §
//! Sub-agents). Fire-and-forget: the caller gets the child's ids immediately
//! and never its result. Designed to be called by the model through
//! `agent_trigger`; the dispatch layer records the child linkage on a `Done`
//! checkpoint (creation count, status, stop cascade).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::MessageInput;
use crate::prompt::{Mode, SystemPromptStrategy};
use crate::types::model::ThinkingLevel;
use crate::types::output::OutputContract;
use crate::types::turn::FunctionPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubagentIcon {
    Agent,
    Code,
    Search,
    Terminal,
    Database,
    Test,
    Review,
    Docs,
    Design,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubagentColor {
    Neutral,
    Blue,
    Purple,
    Teal,
    Green,
    Amber,
    Rose,
}

/// Display-only identity for a spawned child. The name becomes the session
/// title; icon and color are closed semantic tokens consumed by UIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubagentDisplay {
    /// Short functional name, such as `Frontend` or `Explorer`. Leading and
    /// trailing whitespace is removed; the result must be 1-48 characters.
    #[schemars(length(min = 1, max = 48))]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<SubagentIcon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<SubagentColor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SpawnOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// How `system_prompt` combines with the built-in prompt: `override`
    /// replaces it; `enrich` (default) appends to it; `disabled` omits it.
    #[serde(default)]
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// Capped at the parent's remaining turn budget. Omit unless a strict
    /// child-specific cap is required. It must cover discovery/contract calls
    /// plus every work call; very small values (for example 2-3) commonly
    /// strand the child before it can produce its deliverable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Inherits the parent's ceiling unless explicitly narrowed/overridden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// The child's deliverable: text / json / json+schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputContract>,
    /// Override the child's validation-retry budget (output contract AND
    /// `harness::hook::post-turn` deny re-prompts). Default: worker config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_validation_retries: Option<u32>,
    /// Intersected with the parent policy — narrow, never escalate. An
    /// `ask`-mode child is further capped at the configured default policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// Exact skill ids advertised to the child. On a fresh child, omitted or
    /// empty means all. A reused child inherits when omitted and resets to all
    /// when empty. Explicit changes require no active child turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Grant this child the orchestration surface. Default false: a spawned
    /// child is a LEAF — its policy gains deny globs for `harness::spawn`,
    /// `harness::send`, `engine::register_trigger`, `engine::unregister_trigger`
    /// and `engine::registered-triggers::*`, so it performs its assignment and
    /// updates shared state without spawning, messaging sessions, or touching
    /// trigger registrations. `true` skips those denies; the child still never
    /// exceeds its parent's policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<bool>,
    /// Fan-out guard for the child's own spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_children: Option<u32>,
    /// Absolute filesystem root for the child turn (e.g. an isolated
    /// `worktree::create` checkout), written to the child's
    /// `metadata.fs_scope.root`. When set it overrides the inherited scope
    /// for this child; when absent the child inherits its direct parent's
    /// root unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnRequest {
    /// The child's self-contained goal — its opening user message. Include
    /// every resolved required selector literally (for example `Use database
    /// db: "primary"`); the child cannot infer resources from the parent.
    pub task: MessageInput,
    /// Run the child as a directory agent profile (`directory::agents::*`
    /// id). The profile's body becomes the child's enrich system prompt
    /// (over the shared identity), its skill
    /// filter applies when `options.skills` is omitted, its `model` slots
    /// between an explicit `model` and the parent's, and its name/icon become
    /// the display defaults. Which agent profile to name is the prompt's decision.
    /// Refused combined with `options.system_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Optional display-only identity for the child session. This never affects
    /// session ids, policy, routing, or execution. On named-session reuse the
    /// existing session title and metadata are retained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<SubagentDisplay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Spawn into this session, creating it if it does not exist (e.g. a fork,
    /// or a pre-chosen id to filter `turn-completed` subscriptions on); default:
    /// create fresh. An in-turn spawn may reuse an EXISTING id only inside its
    /// own tree (itself, or a child it spawned) — anything else is refused as a
    /// cross-run id collision. The response reports `reused: true` on reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Display-only parent for the console session tree, used when there is no
    /// live parent turn (e.g. a console- or workflow-issued spawn).
    /// Writes `SessionMeta.metadata.parent_session_id` so the console nests this
    /// child; it does NOT grant policy inheritance or parent-call resolution.
    /// Ignored when the dispatcher injects a real parent link (an in-turn spawn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SpawnOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnResponse {
    pub child_session_id: String,
    pub child_turn_id: String,
    /// The named session already existed and was reused — its prior transcript
    /// and parent linkage were retained (only possible with an explicit
    /// `session_id`).
    #[serde(default)]
    pub reused: bool,
}

/// Direct-call entry (a consumer starting a linked child). Dispatched from a
/// turn, the dispatch layer handles linkage + guards; here we start a child
/// and return its ids. Parent linkage is injected by the dispatcher, never
/// trusted from model arguments — a direct call has no parent.
pub async fn handle(deps: &Deps, req: SpawnRequest) -> Result<SpawnResponse, HarnessError> {
    let ids = crate::subagent::spawn_child(deps, &req, None).await?;
    Ok(SpawnResponse {
        child_session_id: ids.session_id,
        child_turn_id: ids.turn_id,
        reused: ids.reused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spawn_skills_option_is_ids_only_on_the_wire() {
        let value = serde_json::to_value(SpawnOptions {
            skills: Some(vec!["review".into()]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(value["skills"], json!(["review"]));
    }

    #[test]
    fn orchestrator_round_trips_unset_and_both_values() {
        let unset: SpawnOptions = serde_json::from_value(json!({})).unwrap();
        assert_eq!(unset.orchestrator, None);
        assert!(!serde_json::to_value(&unset)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("orchestrator"));
        for value in [true, false] {
            let opts: SpawnOptions =
                serde_json::from_value(json!({ "orchestrator": value })).unwrap();
            assert_eq!(opts.orchestrator, Some(value));
        }
    }

    #[test]
    fn display_tokens_are_closed_on_the_wire() {
        let request: SpawnRequest = serde_json::from_value(json!({
            "task": "build the interface",
            "display": { "name": "Frontend", "icon": "code", "color": "blue" }
        }))
        .unwrap();
        let display = request.display.unwrap();
        assert_eq!(display.name, "Frontend");
        assert_eq!(display.icon, Some(SubagentIcon::Code));
        assert_eq!(display.color, Some(SubagentColor::Blue));

        assert!(serde_json::from_value::<SpawnRequest>(json!({
            "task": "x",
            "display": { "name": "Unsafe", "icon": "<svg>" }
        }))
        .is_err());
        assert!(serde_json::from_value::<SpawnRequest>(json!({
            "task": "x",
            "display": { "name": "Unsafe", "color": "#ff00ff" }
        }))
        .is_err());
    }
}
