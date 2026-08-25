//! `directory::pre-generate` — the conditional search hint. One short
//! standing instruction pointing the model at `directory::search_functions`,
//! injected at most once per turn and only when discovery is plausibly
//! needed. Moved from the reflex spike's `search` selector branch; measured
//! there: an unconditional per-generation hint *induces* redundant discovery
//! on guided tasks (double discovery, up to +110% tokens), so every gate
//! below earns its place.

use serde_json::Value;

use crate::functions::search::{Deps, HintRecord};
use crate::functions::search_index::ToolSchema;

/// Text-extraction for guided-task detection: mirrors the message shape the
/// harness sends (content blocks of `{type: "text", text}`).
fn text_content(message: &Value) -> Option<String> {
    let text = message
        .get("content")?
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

/// The live pre-generate binding, if any. Shared with the configuration
/// change handler so an `inject_hint` flip can bind/unbind at runtime.
pub type HintBindingState = iii_config_client::BindingSlot;

/// Reconcile the live binding with the configured `inject_hint` value:
/// on → bind once; off → unregister and drop the handle. Idempotent under
/// repeated config events, and a failed bind retries on the next event.
/// If the harness is not up yet the engine parks the binding as a pending
/// intent and activates it when the trigger type registers (recoverable
/// triggers). `on_error: fail_open` is mandatory: pre_generate defaults
/// fail-CLOSED, and a missing hint must never block a turn.
pub fn apply(iii: &iii_sdk::IIIClient, state: &HintBindingState, enabled: bool) {
    state.reconcile(
        enabled,
        || {
            iii_config_client::try_bind(
                iii,
                iii_sdk::protocol::RegisterTriggerInput::new(
                    "harness::hook::pre-generate",
                    "directory::pre-generate",
                    serde_json::json!({ "priority": 100, "timeout_ms": 5000, "on_error": "fail_open" }),
                ),
            )
        },
        "inject_hint on: the search hint can join agent generations",
        "inject_hint off: the search hint stays out of agent generations",
    );
}

#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct PreGenerateHookRequest {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub depth: u32,
    pub generate: GeneratePayload,
}

#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct GeneratePayload {
    pub system_prompt: String,
    pub messages: Vec<Value>,
    pub functions_generation: u64,
    pub tools: Vec<ToolSchema>,
    /// How the harness exposes functions to the model for this generation.
    /// Absent on harnesses that predate the field, which only ever exposed
    /// through agent_trigger by default.
    #[serde(default)]
    pub expose: ExposeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExposeKind {
    #[default]
    AgentTrigger,
    Native,
    /// Forward compatibility: an exposure mode this build does not know gets
    /// the mode-neutral call instruction rather than a deserialization error.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct PreGenerateMutations {
    pub system_prompt: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct PreGenerateHookResponse {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutations: Option<PreGenerateMutations>,
    pub annotations: PreGenerateAnnotations,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct PreGenerateAnnotations {
    pub directory: DiscoveryTranscriptAnnotation,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct DiscoveryTranscriptAnnotation {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u8,
    pub summary: String,
    pub data: DiscoveryPassV1,
}

/// The durable transcript row for one hook pass. Coarse by design: outcome,
/// reason, and counts only — no prompts, messages, tool contracts, session
/// ids, or timings ever leave the hook.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct DiscoveryPassV1 {
    pub outcome: DiscoveryOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<DiscoveryReason>,
    pub allowed_functions: u64,
    pub functions_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryOutcome {
    HintInjected,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryReason {
    SearchUnavailable,
    AlreadySearched,
    NarrowSurface,
    AlreadyOperating,
    TaskGuided,
    HintAlreadySent,
}

const TRANSCRIPT_ANNOTATION_TYPE: &str = "console.transcript";
const PASS_VERSION: u8 = 1;

fn annotation(data: DiscoveryPassV1) -> PreGenerateAnnotations {
    let summary = match data.outcome {
        DiscoveryOutcome::HintInjected => "directory · hint injected".to_string(),
        DiscoveryOutcome::Skipped => "directory · skipped".to_string(),
    };
    PreGenerateAnnotations {
        directory: DiscoveryTranscriptAnnotation {
            kind: TRANSCRIPT_ANNOTATION_TYPE.to_string(),
            version: PASS_VERSION,
            summary,
            data,
        },
    }
}

fn skipped(
    reason: DiscoveryReason,
    allowed_functions: u64,
    functions_generation: u64,
) -> PreGenerateHookResponse {
    PreGenerateHookResponse {
        decision: "continue".to_string(),
        mutations: None,
        annotations: annotation(DiscoveryPassV1 {
            outcome: DiscoveryOutcome::Skipped,
            reason: Some(reason),
            allowed_functions,
            functions_generation,
        }),
    }
}

/// True when the surface exposes fewer than `minimum` distinct non-engine
/// workers: on a trivial surface normal discovery is cheaper than a
/// standing hint.
fn narrow_surface(tools: &[ToolSchema], minimum: usize) -> bool {
    let mut namespaces: Vec<&str> = Vec::new();
    for tool in tools {
        // The search itself is not a capability worth hinting about.
        if tool.name == crate::functions::search_index::SEARCH_FN
            || crate::functions::search_index::EXCLUDED_NAMESPACE_PREFIXES
                .iter()
                .any(|prefix| tool.name.starts_with(prefix))
        {
            continue;
        }
        let Some((namespace, _)) = tool.name.split_once("::") else {
            continue;
        };
        if !namespaces.contains(&namespace) {
            namespaces.push(namespace);
        }
    }
    namespaces.len() < minimum
}

/// True when a message AFTER the latest user message is a function_result
/// from a real worker: the task in progress is already exercising working
/// calls, and a discovery hint would only re-open a settled path mid-turn.
/// Evidence from BEFORE the latest user message belongs to the previous
/// task — a new user message opens new capability needs, so it resets the
/// window (otherwise a session whose first turn operated could never be
/// re-hinted, even once the hint record went stale).
fn operating_evidence(messages: &[Value]) -> bool {
    let start = latest_user_index(messages).map_or(0, |i| i + 1);
    messages[start..].iter().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("function_result")
            && message
                .get("function_id")
                .and_then(Value::as_str)
                .is_some_and(|id| {
                    !id.starts_with("engine::") && id != crate::functions::search_index::SEARCH_FN
                })
    })
}

/// Index of the latest user message, if any. Gates that reason about "the
/// current task" scope their evidence from here instead of the whole
/// window: a new user message opens new capability needs, and evidence
/// from the previous task must not silence the hint for the next one.
fn latest_user_index(messages: &[Value]) -> Option<usize> {
    messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
}

/// True when the current task's text (the latest user message onward)
/// names a concrete callable function id from the surface (excluded
/// prefixes aside): the task is guided — its contracts are spelled out or
/// trivially known — and a discovery hint measurably *induces* redundant
/// discovery instead of helping. Candidates are whitespace-split words
/// carrying `::`, trimmed of surrounding punctuation.
fn guided_by_named_functions(messages: &[Value], tools: &[ToolSchema]) -> bool {
    let callable: std::collections::HashSet<&str> = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .filter(|name| {
            *name != crate::functions::search_index::SEARCH_FN
                && crate::functions::search_index::EXCLUDED_NAMESPACE_PREFIXES
                    .iter()
                    .all(|prefix| !name.starts_with(prefix))
        })
        .collect();
    if callable.is_empty() {
        return false;
    }
    let start = latest_user_index(messages).unwrap_or(0);
    messages[start..].iter().any(|message| {
        text_content(message).is_some_and(|text| {
            text.split_whitespace().any(|word| {
                word.contains("::")
                    && callable.contains(word.trim_matches(|c: char| {
                        !c.is_alphanumeric() && c != ':' && c != '_' && c != '-'
                    }))
            })
        })
    })
}

/// The exact `<discovery_assist>` block one generation receives.
fn hint_block(functions_generation: u64, expose: ExposeKind) -> String {
    let call_instruction = match expose {
        ExposeKind::AgentTrigger => {
            "Invoke it through agent_trigger with this call envelope: { \"function\": \"directory::search_functions\", \"description\": \"<short user-facing search description in the user's language>\", \"payload\": { \"capabilities\": [\"<needed capability>\", \"<another needed capability>\"] } }\n"
        }
        ExposeKind::Native | ExposeKind::Other => {
            "Call directory::search_functions directly with its required `capabilities` request field.\n"
        }
    };
    format!(
        "<discovery_assist functions_generation={functions_generation}>\n\
Before calling any task function whose ID has not already been verified in this conversation by a prior search result, contract lookup, or successful call, call directory::search_functions ONCE at this decision point instead of engine::functions::list. Never invent or call an unverified function ID. A clear capability need is not a verified function ID. This search call is fully specified here; do not look up its contract first. Include one to six short, non-overlapping `capabilities` derived from the goal and current execution state, covering all unmet external capabilities. Always write every `capabilities` entry in English, even when the user writes in another language; preserve proper names, URLs, and function IDs. Do not search for intrinsic reasoning, summarization, planning, or formatting, and do not repeat needs already represented in the conversation or already satisfied. Requests to summarize provided text or content are ignored. The result contains candidates, not contracts: choose the smallest candidate set the task needs, then BEFORE their first use call engine::functions::info ONCE with {{ \"function_ids\": [\"<selected id>\", \"<another selected id>\"] }}. A call marked pre-verified by a Harness runtime block or update is already verified; use its exact payload directly and do not search for it or fetch its contract. {call_instruction}\
</discovery_assist>"
    )
}

/// The conditional hint block appended to the system prompt.
fn hint_prompt(system_prompt: &str, functions_generation: u64, expose: ExposeKind) -> String {
    let hint = hint_block(functions_generation, expose);
    if system_prompt.is_empty() {
        hint
    } else {
        format!("{system_prompt}\n\n{hint}")
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct HintPreviewRequest {}

/// The exact hint text per exposure mode, for the configuration UI. The
/// live block's `functions_generation` attribute varies per generation;
/// the preview renders it as 0.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct HintPreviewResponse {
    pub agent_trigger: String,
    pub native: String,
}

pub fn hint_preview() -> HintPreviewResponse {
    HintPreviewResponse {
        agent_trigger: hint_block(0, ExposeKind::AgentTrigger),
        native: hint_block(0, ExposeKind::Native),
    }
}

/// One hook pass: inject the hint when every gate clears, otherwise skip
/// with a stable reason. Pure over the payload's own messages and tools
/// (except the once-per-turn record), so replays and compaction
/// re-derive the right outcome on their own.
pub async fn pre_generate(deps: &Deps, request: PreGenerateHookRequest) -> PreGenerateHookResponse {
    let config = deps.config.load_full();
    let GeneratePayload {
        system_prompt,
        messages,
        functions_generation,
        tools,
        expose,
    } = request.generate;
    let allowed_functions = tools.len() as u64;

    if !tools
        .iter()
        .any(|tool| tool.name == "directory::search_functions")
    {
        return skipped(
            DiscoveryReason::SearchUnavailable,
            allowed_functions,
            functions_generation,
        );
    }
    let task_start = latest_user_index(&messages).map_or(0, |index| index + 1);
    let searched = messages[task_start..].iter().any(|message| {
        message.get("role").and_then(Value::as_str) == Some("function_result")
            && message.get("function_id").and_then(Value::as_str)
                == Some("directory::search_functions")
    });
    if searched {
        return skipped(
            DiscoveryReason::AlreadySearched,
            allowed_functions,
            functions_generation,
        );
    }
    if narrow_surface(&tools, config.hint_min_workers) {
        return skipped(
            DiscoveryReason::NarrowSurface,
            allowed_functions,
            functions_generation,
        );
    }
    if operating_evidence(&messages) {
        return skipped(
            DiscoveryReason::AlreadyOperating,
            allowed_functions,
            functions_generation,
        );
    }
    if guided_by_named_functions(&messages, &tools) {
        return skipped(
            DiscoveryReason::TaskGuided,
            allowed_functions,
            functions_generation,
        );
    }
    // One hint per turn: a same-step replay re-sends, while only a new turn
    // re-anchors a fresh hint. If generation or exposure changes mid-turn,
    // the existing anchor still holds. If compaction later drops the hinted
    // step, the model simply proceeds
    // without a hint — fail-open by design.
    let send = deps
        .sessions
        .lock()
        .expect("session registry")
        .hint_decision(
            &request.session_id,
            HintRecord {
                turn_id: request.turn_id.clone(),
                step: request.step,
                functions_generation,
                expose,
            },
        );
    if !send {
        return skipped(
            DiscoveryReason::HintAlreadySent,
            allowed_functions,
            functions_generation,
        );
    }
    PreGenerateHookResponse {
        decision: "continue".to_string(),
        mutations: Some(PreGenerateMutations {
            system_prompt: hint_prompt(&system_prompt, functions_generation, expose),
        }),
        annotations: annotation(DiscoveryPassV1 {
            outcome: DiscoveryOutcome::HintInjected,
            reason: None,
            allowed_functions,
            functions_generation,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::config::SkillsConfig;
    use crate::functions::registry::RegistryCache;
    use crate::functions::search::Deps;

    fn tool(name: &str) -> ToolSchema {
        ToolSchema {
            name: name.into(),
            description: format!("{name} description"),
            parameters: json!({ "type": "object" }),
        }
    }

    fn wide_tools() -> Vec<ToolSchema> {
        vec![
            tool("directory::search_functions"),
            tool("github::repo::view"),
            tool("state::set"),
        ]
    }

    fn deps() -> Deps {
        Deps {
            config: SkillsConfig::default().into_shared(),
            catalog: Arc::new(tokio::sync::RwLock::new(Arc::new(Vec::new()))),
            sessions: Arc::default(),
            registry_cache: RegistryCache::new(std::time::Duration::from_millis(0)),
        }
    }

    fn request_for(objective: &str, tools: Vec<ToolSchema>) -> PreGenerateHookRequest {
        PreGenerateHookRequest {
            session_id: "session-secret".into(),
            turn_id: "turn-secret".into(),
            step: 7,
            depth: 0,
            generate: GeneratePayload {
                system_prompt: "base discovery prompt".into(),
                messages: vec![json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": objective }]
                })],
                functions_generation: 3,
                tools,
                expose: ExposeKind::AgentTrigger,
            },
        }
    }

    fn assert_skip(response: &PreGenerateHookResponse, reason: DiscoveryReason) {
        assert!(response.mutations.is_none());
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::Skipped
        );
        assert_eq!(response.annotations.directory.data.reason, Some(reason));
    }

    #[tokio::test]
    async fn injects_the_hint_when_every_gate_clears() {
        let deps = deps();
        let response = pre_generate(&deps, request_for("find the stars", wide_tools())).await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
        let prompt = &response.mutations.as_ref().unwrap().system_prompt;
        assert!(prompt.starts_with("base discovery prompt\n\n<discovery_assist"));
        assert!(prompt.contains("call directory::search_functions ONCE"));
        assert!(prompt
            .contains("Before calling any task function whose ID has not already been verified"));
        assert!(prompt.contains("Never invent or call an unverified function ID"));
        assert!(prompt.contains("A clear capability need is not a verified function ID"));
        assert!(prompt.contains("do not look up its contract first"));
        assert!(prompt.contains("\"description\""));
        assert!(!prompt.contains("functions you do not already know"));
        assert!(!prompt.contains("call no discovery at all"));
        assert!(prompt.contains("agent_trigger"));
        assert!(prompt.contains("\"capabilities\""));
        assert!(!prompt.contains("\"query\""));
        assert!(prompt.contains("current execution state"));
        assert!(prompt.contains("all unmet external capabilities"));
        assert!(prompt.contains("one to six short, non-overlapping `capabilities`"));
        assert!(!prompt.contains("For one capability, omit the field"));
        assert!(prompt.contains(
            "Do not search for intrinsic reasoning, summarization, planning, or formatting"
        ));
        assert!(prompt.contains("Requests to summarize provided text or content are ignored"));
        assert!(prompt.contains("Always write every `capabilities` entry in English"));
        assert!(prompt.contains("even when the user writes in another language"));
        assert!(prompt.contains("preserve proper names, URLs, and function IDs"));
        assert!(prompt.contains("do not repeat needs already represented"));
        assert!(prompt.contains("engine::functions::info"));
        assert!(prompt.contains("function_ids"));
        // The annotation carries counts only.
        assert_eq!(response.annotations.directory.data.allowed_functions, 3);
        assert_eq!(response.annotations.directory.data.functions_generation, 3);
        assert_eq!(
            response.annotations.directory.summary,
            "directory · hint injected"
        );
    }

    #[tokio::test]
    async fn discovery_hint_preserves_preverified_system_calls() {
        let deps = deps();
        let mut tools = wide_tools();
        tools.push(tool("directory::skills::get"));
        let mut request = request_for("use the alpha skill", tools);
        request.generate.system_prompt = "<available_skills>\nFor any listed skill not already loaded, call the pre-verified `directory::skills::get` directly with payload `{\"id\":\"<exact id>\"}`.\n- **alpha** — Alpha.\n</available_skills>".into();

        let response = pre_generate(&deps, request).await;

        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
        let prompt = &response.mutations.as_ref().unwrap().system_prompt;
        assert!(prompt.contains("payload `{\"id\":\"<exact id>\"}`"));
        assert!(prompt.contains("A call marked pre-verified by a Harness runtime block or update"));
        assert!(prompt.contains("do not search for it or fetch its contract"));
        assert!(prompt
            .contains("Before calling any task function whose ID has not already been verified"));
        assert!(prompt.contains("then BEFORE their first use call engine::functions::info"));
    }

    #[tokio::test]
    async fn native_exposure_swaps_the_call_instruction() {
        let deps = deps();
        let mut request = request_for("find the stars", wide_tools());
        request.generate.expose = ExposeKind::Native;
        let response = pre_generate(&deps, request).await;
        let prompt = &response.mutations.as_ref().unwrap().system_prompt;
        assert!(prompt.contains("required `capabilities` request field"));
        assert!(!prompt.contains("agent_trigger"));
    }

    #[tokio::test]
    async fn skips_when_search_is_not_callable() {
        let deps = deps();
        let response = pre_generate(
            &deps,
            request_for("find", vec![tool("github::repo::view"), tool("state::set")]),
        )
        .await;
        assert_skip(&response, DiscoveryReason::SearchUnavailable);
    }

    #[tokio::test]
    async fn skips_after_a_search_result_in_the_window() {
        let deps = deps();
        let mut request = request_for("find", wide_tools());
        request.generate.messages.push(json!({
            "role": "function_result",
            "function_id": "directory::search_functions",
            "content": [{ "type": "text", "text": "{\"workers\":[]}" }]
        }));
        let response = pre_generate(&deps, request).await;
        assert_skip(&response, DiscoveryReason::AlreadySearched);
    }

    #[tokio::test]
    async fn a_previous_tasks_search_does_not_suppress_the_new_task_hint() {
        let deps = deps();
        let mut request = request_for("summarize the news", wide_tools());
        request.generate.messages = vec![
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "find repository functions" }]
            }),
            json!({
                "role": "function_result",
                "function_id": "directory::search_functions",
                "content": [{ "type": "text", "text": "{\"workers\":[]}" }]
            }),
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "summarize the news" }]
            }),
        ];

        let response = pre_generate(&deps, request).await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
    }

    #[tokio::test]
    async fn skips_narrow_surfaces() {
        let deps = deps();
        let response = pre_generate(
            &deps,
            request_for(
                "find",
                vec![
                    tool("directory::search_functions"),
                    tool("github::repo::view"),
                ],
            ),
        )
        .await;
        assert_skip(&response, DiscoveryReason::NarrowSurface);
    }

    #[tokio::test]
    async fn skips_once_the_session_is_operating() {
        let deps = deps();
        let mut request = request_for("find", wide_tools());
        request.generate.messages.push(json!({
            "role": "function_result",
            "function_id": "github::repo::view",
            "content": [{ "type": "text", "text": "{\"stargazers_count\":1}" }]
        }));
        let response = pre_generate(&deps, request).await;
        assert_skip(&response, DiscoveryReason::AlreadyOperating);
    }

    #[tokio::test]
    async fn task_guidance_resets_at_the_latest_user_message() {
        // Turn 1 named a function ("use browser::fetch…"), turn 2 does not:
        // the old guidance belongs to the previous task and must not
        // silence the hint for the new one.
        let deps = deps();
        let mut request = request_for("find", wide_tools());
        request.generate.messages = vec![
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "use github::repo::view to check stars" }]
            }),
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "now do something else entirely" }]
            }),
        ];
        let response = pre_generate(&deps, request).await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
    }

    #[tokio::test]
    async fn operating_evidence_resets_at_the_latest_user_message() {
        // Turn 1 operated (real worker result), then a NEW user message
        // arrived: the old evidence is the previous task's and must not
        // block the hint for the new one.
        let deps = deps();
        let mut request = request_for("find the stars", wide_tools());
        request.generate.messages = vec![
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "summarize the news" }]
            }),
            json!({
                "role": "function_result",
                "function_id": "github::repo::view",
                "content": [{ "type": "text", "text": "{\"stargazers_count\":1}" }]
            }),
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "now do something else entirely" }]
            }),
        ];
        let response = pre_generate(&deps, request).await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
    }

    #[tokio::test]
    async fn still_hints_after_engine_discovery_results() {
        let deps = deps();
        let mut request = request_for("find", wide_tools());
        request.generate.messages.push(json!({
            "role": "function_result",
            "function_id": "engine::functions::list",
            "content": [{ "type": "text", "text": "many functions" }]
        }));
        let response = pre_generate(&deps, request).await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
    }

    #[tokio::test]
    async fn skips_when_the_task_names_a_callable_function() {
        let deps = deps();
        let response = pre_generate(
            &deps,
            request_for(
                "Call engine::register_trigger with function_id: \"state::set\" exactly.",
                wide_tools(),
            ),
        )
        .await;
        assert_skip(&response, DiscoveryReason::TaskGuided);
    }

    #[tokio::test]
    async fn hints_when_named_functions_are_off_the_surface() {
        let deps = deps();
        let response = pre_generate(
            &deps,
            request_for("use database::query for this task", wide_tools()),
        )
        .await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
    }

    #[tokio::test]
    async fn hint_fires_once_per_turn_but_repeats_on_same_step_replays() {
        let deps = deps();
        let first = pre_generate(&deps, request_for("find", wide_tools())).await;
        assert_eq!(
            first.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
        let replay = pre_generate(&deps, request_for("find", wide_tools())).await;
        assert_eq!(
            replay.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
        let mut later = request_for("find", wide_tools());
        later.step += 1;
        let response = pre_generate(&deps, later).await;
        assert_skip(&response, DiscoveryReason::HintAlreadySent);
    }

    #[tokio::test]
    async fn a_new_turn_rearms_a_hint_ignored_on_a_greeting() {
        let deps = deps();
        let greeting = pre_generate(&deps, request_for("hey there", wide_tools())).await;
        assert_eq!(
            greeting.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );

        let mut task = request_for("summarize the latest news", wide_tools());
        task.turn_id = "next-turn".into();
        task.generate.messages = vec![
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "hey there" }]
            }),
            json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": "Hello!" }]
            }),
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "summarize the latest news" }]
            }),
        ];

        let response = pre_generate(&deps, task).await;
        assert_eq!(
            response.annotations.directory.data.outcome,
            DiscoveryOutcome::HintInjected
        );
    }
}
