//! The `harness::hook::pre-generate` handler.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::registry::{Provider, ProviderRegistry};
use super::render::{
    advertised_providers, already_resolved, mentions_block, pending_mentions, providers_block,
    MentionStatus,
};
use super::{resolve, BLOCK_BUDGET_CHARS, MAX_MENTIONS};

/// The envelope the harness posts to pre-generate hooks (only what this
/// hook reads; everything else is ignored).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub step: u64,
    #[serde(default)]
    pub generate: Option<GenerateInput>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateInput {
    /// The window this step sends, earlier hook notices replayed in place.
    #[serde(default)]
    pub messages: Vec<Value>,
    /// The concrete functions this session may call (`{ name, … }`).
    #[serde(default)]
    pub tools: Vec<Value>,
}

/// Always `continue`: mentions enrich a turn, they never gate it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HookResponse {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutations: Option<Mutations>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Map<String, Value>>,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct Mutations {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub append_messages: Vec<Value>,
}

impl HookResponse {
    fn pass() -> Self {
        Self {
            decision: "continue".into(),
            mutations: None,
            annotations: None,
        }
    }
}

/// What a generation needs from this hook, decided from its messages.
#[derive(Debug, PartialEq)]
pub struct Plan {
    /// The `<mention_providers>` index is missing or out of date.
    pub advertise: bool,
    /// Mentions no earlier `<mentions>` block resolved.
    pub pending: Vec<mention_contract::MentionRef>,
}

pub fn plan(messages: &[Value], providers: &[Provider]) -> Plan {
    let names: BTreeSet<String> = providers
        .iter()
        .map(|p| p.descriptor.name.clone())
        .collect();
    let advertise = !names.is_empty() && advertised_providers(messages).as_ref() != Some(&names);
    let pending = pending_mentions(messages, &already_resolved(messages), MAX_MENTIONS);
    Plan { advertise, pending }
}

fn tool_names(tools: &[Value]) -> HashSet<String> {
    tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A user-role text message for `append_messages` (`timestamp` is required
/// by the router's message types and never reaches the provider).
fn user_text_message(text: String) -> Value {
    json!({
        "role": "user",
        "content": [{ "type": "text", "text": text }],
        "timestamp": now_ms(),
    })
}

pub async fn pre_generate(
    iii: &Arc<IIIClient>,
    registry: &ProviderRegistry,
    input: PreGenerateInput,
) -> HookResponse {
    let Some(generate) = input.generate else {
        return HookResponse::pass();
    };
    let providers = registry.providers(iii).await;
    let plan = plan(&generate.messages, &providers);
    if !plan.advertise && plan.pending.is_empty() {
        return HookResponse::pass();
    }

    let mut mutations = Mutations::default();
    let mut annotations = Map::new();
    if plan.advertise {
        mutations
            .append_messages
            .push(user_text_message(providers_block(&providers)));
        annotations.insert("mention_providers".into(), json!(providers.len()));
    }
    if !plan.pending.is_empty() {
        let allowed = tool_names(&generate.tools);
        let resolved = resolve(iii, &providers, &plan.pending, Some(&allowed)).await;
        let count = resolved
            .iter()
            .filter(|m| m.status == MentionStatus::Resolved)
            .count();
        annotations.insert("mentions_resolved".into(), json!(count));
        annotations.insert(
            "mentions".into(),
            json!(resolved
                .iter()
                .map(|m| json!({ "name": m.name, "id": m.id, "status": m.status }))
                .collect::<Vec<_>>()),
        );
        mutations
            .append_messages
            .push(user_text_message(mentions_block(
                &resolved,
                BLOCK_BUDGET_CHARS,
            )));
    }
    tracing::debug!(
        session_id = %input.session_id,
        step = input.step,
        appended = mutations.append_messages.len(),
        "mention context appended"
    );
    HookResponse {
        decision: "continue".into(),
        mutations: Some(mutations),
        annotations: Some(annotations),
    }
}
