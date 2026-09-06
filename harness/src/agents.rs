//! Agent-profile resolution (`directory::agents::*`): `options.agent` on
//! `harness::send` and `agent` on `harness::spawn` name a filesystem-backed
//! profile served by the iii-directory worker. The profile is fetched ONCE
//! here and frozen onto the turn (identity, prompt, skills, model, display) —
//! later directory edits never reach a live session, matching the skills
//! baseline freeze. The directory serves the prompt already resolved
//! (`extends` chains composed root-first), and under a profile that prompt
//! IS the session identity: nothing built-in sits underneath it.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Value};

use iii_sdk::protocol::TriggerRequest;

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::spawn::{SubagentColor, SubagentIcon};
use crate::types::model::ThinkingLevel;
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
    reasoning_effort: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    color: Option<String>,
    /// Set by the directory when the profile's `extends` chain does not
    /// resolve; the served prompt is then the file's own body only.
    #[serde(default)]
    inheritance_error: Option<String>,
}

/// A profile resolved and normalized for turn seeding.
#[derive(Debug, Clone)]
pub struct ResolvedAgent {
    /// Frozen onto `TurnOptions.agent`.
    pub identity: AgentIdentity,
    /// The profile's resolved system prompt, verbatim — the whole identity
    /// of a session running as this agent (nothing built-in underneath).
    pub prompt: String,
    /// `None` when the profile filters nothing (every skill).
    pub skills: Option<Vec<String>>,
    /// Authoritative model for sessions running as this agent when present.
    pub model: Option<String>,
    /// Provider-native reasoning effort paired with the profile model.
    pub reasoning_effort: Option<String>,
    /// Display name for spawn identity defaults.
    pub name: String,
    /// Harness display icon; `None` when the profile has none (the token set
    /// is shared, so a directory-validated icon always parses).
    pub icon: Option<SubagentIcon>,
    /// Harness display color; `None` when the profile uses the neutral
    /// default.
    pub color: Option<SubagentColor>,
}

/// Fetch and normalize one agent profile. An unknown id or a profile whose
/// `extends` chain does not resolve maps to `InvalidRequest` (the directory's
/// D41x messages already carry the did-you-mean and next-action hints); any
/// other failure is `Dependency`.
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
    check_resolvable(&wire)?;
    Ok(normalize(id, wire))
}

/// A profile whose `extends` chain is broken is served with its own body
/// only, plus the directory's D415 explanation. Running it would silently
/// drop the identity it was written to build on, so it is refused as the
/// caller's error — the message names the fix.
fn check_resolvable(wire: &AgentGetWire) -> Result<(), HarnessError> {
    match wire
        .inheritance_error
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        Some(message) => Err(HarnessError::InvalidRequest(format!(
            "agent profile resolution failed: {message}"
        ))),
        None => Ok(()),
    }
}

/// D41x is the directory's agent-profile error family (D410 not found, D414
/// write conflicts) — the caller named a bad profile, not a broken
/// dependency.
fn classify_fetch_error(message: &str) -> HarnessError {
    if message.contains("D41") {
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
    let icon = wire
        .icon
        .and_then(|token| serde_json::from_value::<SubagentIcon>(Value::String(token)).ok());
    let color = wire
        .color
        .and_then(|token| serde_json::from_value::<SubagentColor>(Value::String(token)).ok());
    ResolvedAgent {
        identity: AgentIdentity {
            id: id.to_string(),
            name: Some(name.clone()),
            icon: icon.and_then(|value| {
                serde_json::to_value(value)
                    .ok()?
                    .as_str()
                    .map(str::to_string)
            }),
            color: color.and_then(|value| {
                serde_json::to_value(value)
                    .ok()?
                    .as_str()
                    .map(str::to_string)
            }),
        },
        prompt: wire.system_prompt,
        skills: (!wire.skills.is_empty()).then_some(wire.skills),
        model: wire.model,
        reasoning_effort: wire.reasoning_effort,
        name,
        icon,
        color,
    }
}

impl ResolvedAgent {
    /// Split the Console catalog key (`provider::model`) when present. Plain
    /// router model ids remain valid and leave provider routing automatic.
    pub fn model_and_provider(&self) -> Option<(String, Option<String>)> {
        let model = self.model.as_deref()?.trim();
        let split = model
            .split_once("::")
            .filter(|(provider, id)| !provider.is_empty() && !id.is_empty());
        Some(match split {
            Some((provider, id)) => (id.to_string(), Some(provider.to_string())),
            None => (model.to_string(), None),
        })
    }

    /// Apply the profile's effort as both the compatibility enum (when it is
    /// one of the Harness levels) and the exact provider-native option. The
    /// latter preserves catalog additions such as `ultra` without a Harness
    /// enum release.
    pub fn apply_reasoning(
        &self,
        provider: Option<&str>,
        thinking_level: &mut Option<ThinkingLevel>,
        provider_options: &mut Option<BTreeMap<String, Value>>,
    ) {
        let Some(effort) = self
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|effort| !effort.is_empty() && *effort != "default")
        else {
            return;
        };
        *thinking_level = serde_json::from_value(Value::String(effort.to_lowercase())).ok();
        let Some(provider) = provider else {
            return;
        };
        let options = provider_options.get_or_insert_with(BTreeMap::new);
        let provider_value = options
            .entry(provider.to_string())
            .or_insert_with(|| json!({}));
        if !provider_value.is_object() {
            *provider_value = json!({});
        }
        provider_value
            .as_object_mut()
            .expect("provider options normalized to an object")
            .insert("reasoning_effort".into(), Value::String(effort.to_string()));
    }

    /// Frozen session-manager metadata consumed by Console and other clients.
    pub fn session_metadata(&self) -> Value {
        let mut value =
            serde_json::to_value(&self.identity).expect("agent identity always serializes");
        let object = value
            .as_object_mut()
            .expect("agent identity serializes as an object");
        if let Some(model) = &self.model {
            object.insert("model".into(), Value::String(model.clone()));
        }
        if let Some(reasoning_effort) = &self.reasoning_effort {
            object.insert(
                "reasoning_effort".into(),
                Value::String(reasoning_effort.clone()),
            );
        }
        value
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

        let err = classify_fetch_error(
            "handler error: D415 invalid_input: agent profile \"lead\" extends unknown agent \
             profile \"nope\". Next: call directory::agents::list to browse agent profile ids.",
        );
        assert_eq!(err.code(), "harness/invalid_request");

        let err = classify_fetch_error("dispatch timed out");
        assert_eq!(err.code(), "harness/dependency");
        assert!(err.to_string().contains(AGENTS_GET_ID));
    }

    /// The directory serves a broken chain fail-soft (own body + D415) so its
    /// editor can open the profile; the harness must not run that half
    /// identity.
    #[test]
    fn broken_inheritance_chain_is_refused_as_invalid_request() {
        let broken = wire(serde_json::json!({
            "name": "Lead",
            "system_prompt": "Own body only.",
            "inheritance_error": "D415 invalid_input: agent profile \"lead\" extends unknown agent profile \"nope\".",
        }));
        let err = check_resolvable(&broken).unwrap_err();
        assert_eq!(err.code(), "harness/invalid_request");
        assert!(err.to_string().contains("extends unknown agent profile"));

        let fine = wire(serde_json::json!({ "name": "Lead", "system_prompt": "Body." }));
        assert!(check_resolvable(&fine).is_ok());
        let blank = wire(serde_json::json!({
            "name": "Lead",
            "system_prompt": "Body.",
            "inheritance_error": "  ",
        }));
        assert!(check_resolvable(&blank).is_ok());
    }

    #[test]
    fn normalize_keeps_the_resolved_prompt_verbatim_and_optionalizes_fields() {
        let agent = normalize(
            "tech-leader",
            wire(serde_json::json!({
                "name": "Tech Leader",
                "system_prompt": "Delegate everything.",
                "skills": [],
                "model": "openai-codex::codex/gpt-5.4",
                "reasoning_effort": "ultra",
                "icon": "agent",
                "color": "purple",
            })),
        );
        assert_eq!(
            agent.prompt, "Delegate everything.",
            "the directory's resolved prompt is the identity — no prefix"
        );
        assert_eq!(agent.skills, None, "empty filter means every skill");
        assert_eq!(agent.identity.id, "tech-leader");
        assert_eq!(agent.identity.name.as_deref(), Some("Tech Leader"));
        assert_eq!(agent.identity.icon.as_deref(), Some("agent"));
        assert_eq!(agent.identity.color.as_deref(), Some("purple"));
        assert_eq!(agent.icon, Some(SubagentIcon::Agent));
        assert_eq!(agent.color, Some(SubagentColor::Purple));
        assert_eq!(
            agent.model_and_provider(),
            Some(("codex/gpt-5.4".into(), Some("openai-codex".into())))
        );
        assert_eq!(agent.reasoning_effort.as_deref(), Some("ultra"));
        assert_eq!(
            agent.session_metadata(),
            serde_json::json!({
                "id": "tech-leader",
                "name": "Tech Leader",
                "icon": "agent",
                "color": "purple",
                "model": "openai-codex::codex/gpt-5.4",
                "reasoning_effort": "ultra",
            })
        );

        let mut thinking = Some(ThinkingLevel::Low);
        let mut provider_options = None;
        agent.apply_reasoning(Some("openai-codex"), &mut thinking, &mut provider_options);
        assert_eq!(thinking, None, "native-only effort has no enum fallback");
        assert_eq!(
            provider_options.unwrap()["openai-codex"],
            serde_json::json!({ "reasoning_effort": "ultra" })
        );
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
        assert_eq!(agent.prompt, "Write code.");
        assert_eq!(agent.identity.name.as_deref(), Some("coder"));
        assert_eq!(agent.skills.as_deref(), Some(&["review".to_string()][..]));
        assert_eq!(agent.icon, None, "unknown token degrades, never errors");
        assert_eq!(agent.color, None, "unknown color degrades, never errors");
    }
}
