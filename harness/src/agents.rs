//! Agent-profile resolution (`directory::agents::*`): `options.agent` on
//! `harness::send` and `agent` on `harness::spawn` name a filesystem-backed
//! profile served by the iii-directory worker. The profile is fetched ONCE
//! here and frozen onto the turn (identity, prompt, skills, preloaded functions,
//! model, display) — later directory edits never reach a live session,
//! matching the skills baseline freeze. The directory serves the prompt
//! already resolved (`extends` chains composed root-first), and under a
//! profile that prompt IS the session identity: nothing built-in sits
//! underneath it.
//!
//! A profile's `functions` — its PRELOADED functions — are engine function ids
//! whose contracts the session should not have to discover: this module
//! renders each one's current description and compacted request schema into
//! a `<preloaded_functions>` block appended to the frozen prompt, so the model
//! calls them on the first step instead of spending a search and a contract
//! lookup per session (see [`render_preloaded_functions`]). A profile's
//! `skills` are PRELOADED the same way: each id's body is fetched once from
//! `directory::skills::get` and frozen into a `<preloaded_skills>` block of
//! `<skill id="…">` sections (see [`render_preloaded_skills`]) — the skill is
//! already in context, never something the model has to look up. Both lists
//! arrive from the directory already unioned root-first through `extends`.

use std::collections::{BTreeMap, HashMap};

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
const FUNCTIONS_INFO_ID: &str = "engine::functions::info";
const SKILLS_GET_ID: &str = crate::skills::SKILLS_GET_ID;
/// The engine's `function_ids` batch cap (`engine::functions::info`).
const INFO_BATCH_MAX: usize = 32;

/// The wire subset of `directory::agents::get` the harness consumes; unknown
/// fields are ignored so directory additions never break resolution.
#[derive(Debug, Deserialize)]
struct AgentGetWire {
    name: String,
    system_prompt: String,
    #[serde(default)]
    skills: Vec<String>,
    /// Preloaded function ids, already resolved through `extends` by the
    /// directory; older directories omit the field.
    #[serde(default)]
    functions: Vec<String>,
    /// `skills` entries the directory already knows it cannot serve — they
    /// render as unavailable without a `directory::skills::get` round-trip
    /// (which would fail, and paint an error span on the session's send).
    #[serde(default)]
    unknown_skills: Vec<String>,
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
    /// The profile's preloaded skill ids, in declaration order (the
    /// directory's root-first union through `extends`) — the bodies rendered
    /// into `prompt` by [`resolve`] (ids the directory could not serve are
    /// still listed here; the prompt names them as unavailable). Empty = the
    /// profile declares none. Never a filter: the session's skill index
    /// stays whole, only an explicit `options.skills` narrows it.
    pub skills: Vec<String>,
    /// The profile's preloaded function ids, in declaration order — the
    /// contracts rendered into `prompt` by [`resolve`] (ids the engine did
    /// not know at resolution are still listed here; the prompt names them
    /// as unavailable). Empty = the profile declares none.
    pub functions: Vec<String>,
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
    let unknown_skills = wire.unknown_skills.clone();
    let mut agent = normalize(id, wire);
    attach_preloaded_functions(deps, &mut agent).await;
    attach_preloaded_skills(deps, &mut agent, &unknown_skills).await;
    Ok(agent)
}

/// One preloaded skill as rendered into the prompt: the directory's current
/// body for the id (`directory::skills::get` serves it with the frontmatter
/// already stripped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreloadedSkill {
    pub id: String,
    pub body: String,
}

/// Freeze the profile's preloaded skills onto the prompt: one
/// `directory::skills::get` per id, rendered as `<skill id="…">` sections of
/// a `<preloaded_skills>` block after the identity (and after the preloaded
/// functions). Ids the directory cannot serve — the ones it already reported
/// in `unknown_skills` (skipped without a call), a hidden or renamed id, or
/// a failed call — are named in the block as unavailable so the model never
/// hunts for them. Best effort and resolved ONCE, like the rest of the
/// identity: a skill edited mid-session reaches the next session, not this
/// one.
// ponytail: one sequential bus call per id — a profile preloads a handful; batch in the directory if that stops being true
async fn attach_preloaded_skills(
    deps: &Deps,
    agent: &mut ResolvedAgent,
    unknown_skills: &[String],
) {
    if agent.skills.is_empty() {
        return;
    }
    let cfg = deps.cfg().await;
    let mut loaded = Vec::with_capacity(agent.skills.len());
    let mut unavailable = Vec::new();
    for id in &agent.skills {
        if unknown_skills.contains(id) {
            unavailable.push(id.clone());
            continue;
        }
        let response = deps
            .iii
            .trigger(TriggerRequest {
                function_id: SKILLS_GET_ID.into(),
                payload: json!({ "id": id }),
                action: None,
                timeout_ms: Some(cfg.dispatch_timeout_ms),
            })
            .await;
        match response {
            Ok(value) => match skill_in_get_response(id, &value) {
                Some(skill) => loaded.push(skill),
                None => unavailable.push(id.clone()),
            },
            Err(error) => {
                tracing::warn!(
                    agent = %agent.identity.id,
                    skill_id = %id,
                    %error,
                    "preloaded skill could not be fetched; it renders as unavailable"
                );
                unavailable.push(id.clone());
            }
        }
    }
    if !unavailable.is_empty() {
        tracing::warn!(
            agent = %agent.identity.id,
            unavailable = ?unavailable,
            "agent profile names preloaded skills the directory does not serve"
        );
    }
    agent.prompt = append_block(
        &agent.prompt,
        &render_preloaded_skills(&loaded, &unavailable),
    );
}

/// The skill one `directory::skills::get { id }` response carries for the
/// DECLARED id, or `None` when the directory answered with something else —
/// its miss recovery serves a worker or engine overview under a different
/// id, and an overview is not the skill the profile asked to preload. The
/// served id may be the declared one canonicalized (`iii://` prefix and
/// `.md` suffix dropped); the block keeps the profile's spelling, which is
/// what the skills index shows too.
fn skill_in_get_response(id: &str, response: &Value) -> Option<PreloadedSkill> {
    let served = response.get("id").and_then(Value::as_str)?;
    let canonical = id.trim_start_matches("iii://").trim_end_matches(".md");
    if served != id && served != canonical {
        tracing::warn!(
            skill_id = %id,
            served = %served,
            "directory::skills::get answered with a different skill; treating the declared id as unavailable"
        );
        return None;
    }
    let body = response.get("body").and_then(Value::as_str)?.trim();
    (!body.is_empty()).then(|| PreloadedSkill {
        id: id.to_string(),
        body: body.to_string(),
    })
}

/// The `<preloaded_skills>` block: an instruction line, then one
/// `<skill id="…">` section per loaded skill in the profile's declaration
/// order, then the ids the directory could not serve. The `<skill id>` shape
/// is the one the skills index already describes as "already loaded", so
/// the two blocks agree on what the model must not fetch again.
pub(crate) fn render_preloaded_skills(skills: &[PreloadedSkill], unavailable: &[String]) -> String {
    let mut body = String::from(
        "<preloaded_skills>\nThese skills are preloaded for this agent profile: every `<skill id=\"...\">` \
         block below is already in context — follow it directly and do not call \
         `directory::skills::get` for these ids. Every other skill still goes through the skills \
         index and `directory::skills::get`.",
    );
    for skill in skills {
        body.push_str("\n\n<skill id=\"");
        body.push_str(&skill.id);
        body.push_str("\">\n");
        body.push_str(&skill.body);
        body.push_str("\n</skill>");
    }
    if !unavailable.is_empty() {
        body.push_str(
            "\n\nDeclared by the profile but NOT available right now — do not look for them: ",
        );
        body.push_str(
            &unavailable
                .iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        body.push_str(
            ". If the task needs one of them, say so rather than improvising a substitute.",
        );
    }
    body.push_str("\n</preloaded_skills>");
    body
}

/// One preloaded function as rendered into the prompt: the current description
/// and request schema (compacted the way `engine::functions::info` results
/// are compacted for the model — see `trigger::compact_schema`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreloadedContract {
    pub function_id: String,
    pub description: Option<String>,
    pub request_schema: Option<Value>,
}

impl PreloadedContract {
    fn new(function_id: &str, description: Option<&str>, request_schema: Option<Value>) -> Self {
        let mut request_schema = request_schema.filter(|schema| !schema.is_null());
        if let Some(schema) = request_schema.as_mut() {
            crate::trigger::compact_schema(schema);
        }
        Self {
            function_id: function_id.to_string(),
            description: description
                .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|text| !text.is_empty()),
            request_schema,
        }
    }
}

/// Freeze the profile's preloaded functions onto the prompt. Contracts come
/// from the cached registry snapshot (already hydrated with schemas, no
/// round-trip) and, for ids the snapshot cannot vouch for — not listed, or
/// listed without a schema — from one `engine::functions::info` batch per
/// 32 ids. Ids the engine does not know are named in the block as
/// unavailable so the model never dials them blind. Best effort: a profile
/// whose registry lookups all fail still runs, with its declared ids listed
/// as unavailable. Resolved ONCE, like the rest of the identity — a worker
/// that re-registers with a new contract mid-session is what the
/// registry-changed notice covers.
async fn attach_preloaded_functions(deps: &Deps, agent: &mut ResolvedAgent) {
    if agent.functions.is_empty() {
        return;
    }
    let snapshot = deps.functions().await;
    let mut contracts: HashMap<String, PreloadedContract> = HashMap::new();
    let mut pending: Vec<String> = Vec::new();
    for id in &agent.functions {
        match snapshot
            .functions
            .iter()
            .find(|descriptor| descriptor.function_id == *id)
        {
            Some(descriptor) if descriptor.parameters.is_some() => {
                contracts.insert(
                    id.clone(),
                    PreloadedContract::new(
                        id,
                        descriptor.description.as_deref(),
                        descriptor.parameters.clone(),
                    ),
                );
            }
            _ => pending.push(id.clone()),
        }
    }
    let cfg = deps.cfg().await;
    for chunk in pending.chunks(INFO_BATCH_MAX) {
        let response = deps
            .iii
            .trigger(TriggerRequest {
                function_id: FUNCTIONS_INFO_ID.into(),
                payload: json!({ "function_ids": chunk }),
                action: None,
                timeout_ms: Some(cfg.dispatch_timeout_ms),
            })
            .await;
        match response {
            Ok(response) => {
                for contract in contracts_in_info_batch(&response) {
                    contracts.insert(contract.function_id.clone(), contract);
                }
            }
            Err(error) => tracing::warn!(
                agent = %agent.identity.id,
                %error,
                "preloaded function contracts could not be fetched; those ids render as unavailable"
            ),
        }
    }
    let mut ordered = Vec::with_capacity(agent.functions.len());
    let mut unavailable = Vec::new();
    for id in &agent.functions {
        match contracts.remove(id) {
            Some(contract) => ordered.push(contract),
            None => unavailable.push(id.clone()),
        }
    }
    if !unavailable.is_empty() {
        tracing::warn!(
            agent = %agent.identity.id,
            unavailable = ?unavailable,
            "agent profile names preloaded functions the engine does not know"
        );
    }
    agent.prompt = append_block(
        &agent.prompt,
        &render_preloaded_functions(&ordered, &unavailable),
    );
}

/// The contracts one `engine::functions::info { function_ids }` batch
/// returned — not-found markers (`{ function_id, error }`) are skipped, so
/// an id absent from the result is exactly an id the engine does not know.
fn contracts_in_info_batch(response: &Value) -> Vec<PreloadedContract> {
    response
        .get("functions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("error").is_none())
                .filter_map(|item| {
                    let id = item.get("function_id").and_then(Value::as_str)?;
                    Some(PreloadedContract::new(
                        id,
                        item.get("description").and_then(Value::as_str),
                        item.get("request_schema")
                            .or_else(|| item.get("request_format"))
                            .or_else(|| item.get("parameters"))
                            .cloned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The prompt followed by a blank line and the block; a profile with no
/// prompt of its own serves the block alone.
fn append_block(prompt: &str, block: &str) -> String {
    let head = prompt.trim_end_matches('\n');
    if head.is_empty() {
        block.to_string()
    } else {
        format!("{head}\n\n{block}")
    }
}

/// The `<preloaded_functions>` block: an instruction paragraph, then one
/// section per contract — id, one-line description, compact JSON request
/// schema — in the profile's declaration order, then the ids the engine does
/// not know. Read against the doctrine's Step 1 / Step 2: a contract in this
/// block is "pre-verified", so the model skips discovery and
/// `engine::functions::info` for it and calls it on the first step.
pub(crate) fn render_preloaded_functions(
    contracts: &[PreloadedContract],
    unavailable: &[String],
) -> String {
    let mut body = format!(
        "<preloaded_functions>\nThese functions are preloaded for this agent profile: the contracts below are \
         pre-verified and already in context. Call each one directly through `{tool}` with a \
         `payload` object matching its request schema — skip `directory::search_functions` and \
         `engine::functions::info` for these ids (fetch a contract again only after an \
         `invalid_arguments` error or a registry-change notice). Every other function still goes \
         through normal discovery.",
        tool = crate::policy::AGENT_TRIGGER_NAME,
    );
    for contract in contracts {
        body.push_str("\n\n### `");
        body.push_str(&contract.function_id);
        body.push('`');
        if let Some(description) = &contract.description {
            body.push('\n');
            body.push_str(description);
        }
        body.push_str("\nrequest_schema: ");
        match &contract.request_schema {
            Some(schema) => body.push_str(&schema.to_string()),
            None => body.push_str("(none published — call with an empty object `{}` unless the description says otherwise)"),
        }
    }
    if !unavailable.is_empty() {
        body.push_str("\n\nDeclared by the profile but NOT registered right now — do not call: ");
        body.push_str(
            &unavailable
                .iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        body.push_str(
            ". If the task needs one of them, say so rather than improvising a substitute.",
        );
    }
    body.push_str("\n</preloaded_functions>");
    body
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
        skills: wire
            .skills
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect(),
        functions: wire
            .functions
            .into_iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect(),
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
        for (key, ids) in [("skills", &self.skills), ("functions", &self.functions)] {
            if !ids.is_empty() {
                object.insert(
                    key.into(),
                    Value::Array(ids.iter().map(|id| Value::String(id.clone())).collect()),
                );
            }
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
        assert!(
            agent.skills.is_empty(),
            "no skills declared, none preloaded"
        );
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
        assert_eq!(agent.skills, vec!["review"]);
        assert_eq!(agent.icon, None, "unknown token degrades, never errors");
        assert_eq!(
            agent.color,
            Some(SubagentColor::Neutral),
            "unknown color degrades to neutral, never errors"
        );
    }

    #[test]
    fn normalize_carries_preloaded_function_ids_into_metadata() {
        let agent = normalize(
            "engineer",
            wire(serde_json::json!({
                "name": "Engineer",
                "system_prompt": "Write code.",
                "functions": ["coder::tree", " coder::search ", ""],
                "skills": [" review ", "", "deploy"],
            })),
        );
        assert_eq!(agent.functions, vec!["coder::tree", "coder::search"]);
        assert_eq!(agent.skills, vec!["review", "deploy"]);
        assert_eq!(
            agent.session_metadata()["functions"],
            serde_json::json!(["coder::tree", "coder::search"])
        );
        assert_eq!(
            agent.session_metadata()["skills"],
            serde_json::json!(["review", "deploy"])
        );
        // Absent on the wire (older directory) → empty, and no metadata key.
        let plain = normalize(
            "plain",
            wire(serde_json::json!({ "name": "Plain", "system_prompt": "Hi." })),
        );
        assert!(plain.functions.is_empty());
        assert!(plain.session_metadata().get("functions").is_none());
        assert!(plain.session_metadata().get("skills").is_none());
    }

    #[test]
    fn skill_get_response_yields_the_declared_skill_only() {
        let served = serde_json::json!({
            "id": "harness/alpha",
            "title": "Alpha",
            "body": "\n# Alpha\n\nGreet first.\n\n",
            "path": "/skills/harness/alpha.md"
        });
        let skill = skill_in_get_response("harness/alpha", &served).unwrap();
        assert_eq!(skill.id, "harness/alpha");
        assert_eq!(
            skill.body, "# Alpha\n\nGreet first.",
            "trimmed, frontmatter-free body"
        );
        // The declared spelling is kept even when the directory canonicalizes it.
        let canonical = skill_in_get_response("iii://harness/alpha.md", &served).unwrap();
        assert_eq!(canonical.id, "iii://harness/alpha.md");
        // A miss-recovery answer (another id's body) is not the declared skill.
        let overview = serde_json::json!({ "id": "iii/index", "body": "Engine overview." });
        assert!(skill_in_get_response("harness/gone", &overview).is_none());
        // A blank body preloads nothing.
        let blank = serde_json::json!({ "id": "harness/empty", "body": "  \n" });
        assert!(skill_in_get_response("harness/empty", &blank).is_none());
        assert!(skill_in_get_response("harness/alpha", &serde_json::json!({})).is_none());
    }

    #[test]
    fn preloaded_skills_block_lists_bodies_in_order_and_names_the_unavailable() {
        let skills = vec![
            PreloadedSkill {
                id: "harness/alpha".into(),
                body: "# Alpha\n\nGreet first.".into(),
            },
            PreloadedSkill {
                id: "harness/beta".into(),
                body: "Checksum last.".into(),
            },
        ];
        let block = render_preloaded_skills(&skills, &["harness/missing".to_string()]);
        assert!(block.starts_with("<preloaded_skills>\n"));
        assert!(block.ends_with("\n</preloaded_skills>"));
        assert!(block.contains("already in context"));
        assert!(block.contains("do not call `directory::skills::get` for these ids"));
        let alpha = block
            .find("<skill id=\"harness/alpha\">")
            .expect("first skill");
        let beta = block
            .find("<skill id=\"harness/beta\">")
            .expect("second skill");
        assert!(alpha < beta, "declaration order is kept");
        assert!(block.contains(
            "<skill id=\"harness/alpha\">\n# Alpha\n\nGreet first.\n</skill>\n\n<skill id=\"harness/beta\">\nChecksum last.\n</skill>"
        ));
        assert!(
            block.contains("NOT available right now — do not look for them: `harness/missing`.")
        );

        // Nothing unavailable → no such paragraph; the block still explains itself.
        let clean = render_preloaded_skills(&skills, &[]);
        assert!(!clean.contains("NOT available"));
        assert_eq!(clean.matches("</skill>").count(), 2);
    }

    #[test]
    fn info_batch_yields_contracts_and_skips_not_found_markers() {
        let response = serde_json::json!({ "functions": [
            {
                "function_id": "coder::tree",
                "description": "Show a  directory\n tree.",
                "request_schema": {
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "title": "TreeRequest",
                    "type": "object",
                    "properties": { "path": { "type": "string", "default": "." } }
                },
                "response_schema": { "type": "object" },
                "worker_name": "coder"
            },
            { "function_id": "nope::missing", "error": "not_found" },
            { "function_id": "bare", "description": "No schema." }
        ]});
        let contracts = contracts_in_info_batch(&response);
        assert_eq!(contracts.len(), 2, "the marker is not a contract");
        assert_eq!(contracts[0].function_id, "coder::tree");
        assert_eq!(
            contracts[0].description.as_deref(),
            Some("Show a directory tree."),
            "whitespace collapsed"
        );
        let schema = contracts[0].request_schema.as_ref().unwrap();
        assert!(schema.get("$schema").is_none(), "boilerplate stripped");
        assert!(schema.get("title").is_none());
        assert_eq!(schema["properties"]["path"]["type"], "string");
        assert_eq!(
            schema["properties"]["path"]["default"], ".",
            "real defaults survive"
        );
        assert_eq!(contracts[1].function_id, "bare");
        assert!(contracts[1].request_schema.is_none());
        assert!(contracts_in_info_batch(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn preloaded_functions_block_lists_contracts_in_order_and_names_the_unavailable() {
        let contracts = vec![
            PreloadedContract::new(
                "coder::tree",
                Some("Show a directory tree."),
                Some(
                    serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
                ),
            ),
            PreloadedContract::new("bare", None, None),
        ];
        let block = render_preloaded_functions(&contracts, &["gone::away".to_string()]);
        assert!(block.starts_with("<preloaded_functions>\n"));
        assert!(block.ends_with("\n</preloaded_functions>"));
        assert!(block.contains("pre-verified"));
        assert!(block.contains("through `agent_trigger`"));
        let tree = block.find("### `coder::tree`").expect("first contract");
        let bare = block.find("### `bare`").expect("second contract");
        assert!(tree < bare, "declaration order is kept");
        assert!(block.contains(
            "### `coder::tree`\nShow a directory tree.\nrequest_schema: {\"properties\":{\"path\":{\"type\":\"string\"}},\"type\":\"object\"}"
        ));
        assert!(block.contains("### `bare`\nrequest_schema: (none published"));
        assert!(block.contains("NOT registered right now — do not call: `gone::away`."));

        // Nothing unavailable → no such paragraph.
        let clean = render_preloaded_functions(&contracts, &[]);
        assert!(!clean.contains("NOT registered"));

        // The block follows the identity after one blank line, or stands
        // alone for a prompt-less profile.
        assert_eq!(append_block("You lead.\n\n", "<b/>"), "You lead.\n\n<b/>");
        assert_eq!(append_block("", "<b/>"), "<b/>");
    }
}
