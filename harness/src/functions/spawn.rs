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
    /// Short functional name such as `Frontend`, 1-48 characters after trimming.
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
    /// How `system_prompt` combines with the built-in prompt.
    #[serde(default)]
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// Turn cap for the child, capped at the parent's remaining budget; omit
    /// unless required (small values strand the child).
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
    /// Override of the child's validation-retry budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_validation_retries: Option<u32>,
    /// Dispatch policy for the child, intersected with the parent's (narrow,
    /// never escalate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// Exact skill ids advertised to the child; omitted or empty means all (a
    /// reused child inherits when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Let the child spawn, send, and register triggers (still capped by the
    /// parent's policy); default false makes it a leaf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrator: Option<bool>,
    /// Fan-out guard for the child's own spawns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_children: Option<u32>,
    /// Absolute filesystem root for the child turn; omit to inherit the
    /// parent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnRequest {
    /// The child's self-contained goal, its opening user message; name every
    /// required resource selector literally (the child cannot infer them).
    pub task: MessageInput,
    /// Directory agent profile id (`directory::agents::*`) supplying the
    /// child's prompt, skills, model, and display; refused with
    /// `options.system_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Display-only name/icon/color for the child session; never affects ids,
    /// policy, or routing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<SubagentDisplay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Session id to spawn into, created if absent; an existing id may be
    /// reused only inside the caller's own tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Display-only parent for the console tree when there is no live parent
    /// turn; grants no policy inheritance.
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
