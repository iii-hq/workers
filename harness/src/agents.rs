//! Agent-profile resolution (`directory::agents::*`): `options.agent` on
//! `harness::send` and `agent` on `harness::spawn` name a filesystem-backed
//! profile served by the iii-directory worker. The profile is fetched ONCE
//! here and frozen onto the turn (identity, prompt, skills, model, display) —
//! later directory edits never reach a live session, matching the skills
//! baseline freeze.

use serde::Deserialize;
use serde_json::json;

use iii_sdk::protocol::TriggerRequest;

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::spawn::{SubagentColor, SubagentIcon};
use crate::types::turn::AgentIdentity;

const AGENTS_GET_ID: &str = "directory::agents::get";

/// The wire subset of `directory::agents::get` the harness consumes; unknown
/// fields are ignored so directory additions never break resolution.
#[derive(Debug, Deserialize)]
struct AgentGetWire {
    name: String,
    system_prompt: String,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    color: Option<String>,
}

/// A profile resolved and normalized for turn seeding.
#[derive(Debug, Clone)]
pub struct ResolvedAgent {
    /// Frozen onto `TurnOptions.agent`.
    pub identity: AgentIdentity,
    /// `"You are <name>.\n\n<body>"` — the enrich payload.
    pub prompt: String,
    /// `None` when the profile filters nothing (every skill).
    pub skills: Option<Vec<String>>,
    /// Default model for sessions running as this agent.
    pub model: Option<String>,
    /// Display name for spawn identity defaults.
    pub name: String,
    /// Harness display icon; `None` when the profile has none (the token set
    /// is shared, so a directory-validated icon always parses).
    pub icon: Option<SubagentIcon>,
    /// Harness display color; `None` when the profile uses the neutral
    /// default.
    pub color: Option<SubagentColor>,
}

/// Fetch and normalize one agent profile. An unknown id maps to
/// `InvalidRequest` (the directory's D410 message already carries the
/// did-you-mean and next-action hints); any other failure is `Dependency`.
pub async fn resolve(
    deps: &Deps,
    cfg: &WorkerConfig,
    id: &str,
) -> Result<ResolvedAgent, HarnessError> {
    let value = deps
        .iii
        .trigger(TriggerRequest {
            function_id: AGENTS_GET_ID.into(),
            payload: json!({ "id": id }),
            action: None,
            timeout_ms: Some(cfg.dispatch_timeout_ms),
        })
        .await
        .map_err(|e| classify_fetch_error(&e.to_string()))?;
    let wire: AgentGetWire = serde_json::from_value(value).map_err(|e| {
        HarnessError::Dependency(format!("{AGENTS_GET_ID}: malformed response: {e}"))
    })?;
    Ok(normalize(id, wire))
}

/// D410 is the directory's not-found code for agent profiles — the caller named a
/// bad id, not a broken dependency.
fn classify_fetch_error(message: &str) -> HarnessError {
    if message.contains("D410") {
        HarnessError::InvalidRequest(format!("agent profile resolution failed: {message}"))
    } else {
        HarnessError::Dependency(format!("{AGENTS_GET_ID}: {message}"))
    }
}

fn normalize(id: &str, wire: AgentGetWire) -> ResolvedAgent {
    let name = if wire.name.trim().is_empty() {
        id.to_string()
    } else {
        wire.name.trim().to_string()
    };
    ResolvedAgent {
        identity: AgentIdentity { id: id.to_string() },
        prompt: format!("You are {name}.\n\n{}", wire.system_prompt),
        skills: (!wire.skills.is_empty()).then_some(wire.skills),
        model: wire.model,
        name,
        icon: wire.icon.and_then(|t| {
            serde_json::from_value::<SubagentIcon>(serde_json::Value::String(t)).ok()
        }),
        color: wire.color.and_then(|t| {
            serde_json::from_value::<SubagentColor>(serde_json::Value::String(t)).ok()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(json: serde_json::Value) -> AgentGetWire {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn not_found_maps_to_invalid_request_and_keeps_the_directory_hint() {
        let err = classify_fetch_error(
            "handler error: D410 not_found: agent profile \"nope\" does not exist. Did you mean: coder. \
             Next: call directory::agents::list to browse agent profile ids.",
        );
        assert_eq!(err.code(), "harness/invalid_request");
        assert!(err.to_string().contains("directory::agents::list"));

        let err = classify_fetch_error("dispatch timed out");
        assert_eq!(err.code(), "harness/dependency");
        assert!(err.to_string().contains(AGENTS_GET_ID));
    }

    #[test]
    fn normalize_builds_the_you_are_prompt_and_optionalizes_fields() {
        let agent = normalize(
            "tech-leader",
            wire(serde_json::json!({
                "name": "Tech Leader",
                "system_prompt": "Delegate everything.",
                "skills": [],
                "model": "codex/gpt-5.4",
                "icon": "agent",
                "color": "purple",
            })),
        );
        assert_eq!(agent.prompt, "You are Tech Leader.\n\nDelegate everything.");
        assert_eq!(agent.skills, None, "empty filter means every skill");
        assert_eq!(agent.identity.id, "tech-leader");
        assert_eq!(agent.icon, Some(SubagentIcon::Agent));
        assert_eq!(agent.color, Some(SubagentColor::Purple));
        assert_eq!(agent.model.as_deref(), Some("codex/gpt-5.4"));
    }

    #[test]
    fn normalize_survives_blank_name_and_unknown_icon() {
        let agent = normalize(
            "coder",
            wire(serde_json::json!({
                "name": "  ",
                "system_prompt": "Write code.",
                "skills": ["review"],
                "icon": "magnifier",
                "color": "ultraviolet",
            })),
        );
        assert_eq!(
            agent.name, "coder",
            "blank display name falls back to the id"
        );
        assert_eq!(agent.prompt, "You are coder.\n\nWrite code.");
        assert_eq!(agent.skills.as_deref(), Some(&["review".to_string()][..]));
        assert_eq!(agent.icon, None, "unknown token degrades, never errors");
        assert_eq!(agent.color, None, "unknown color degrades, never errors");
    }
}
