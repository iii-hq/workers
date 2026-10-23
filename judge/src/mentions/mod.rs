//! Worker-defined chat mentions, delivered to agents out of the box.
//!
//! A user message may carry `@<name>(id="<id>")` tokens (a kanban ticket, a
//! session, a trace…; see `crates/mention-contract`). The harness always
//! runs this worker alongside it, so the judge hosts the piece that makes
//! agents understand them:
//!
//! - `judge::mentions::pre-generate`, bound to `harness::hook::pre-generate`,
//!   appends a `<mention_providers>` index once per session (which names an
//!   agent may write) and, for mentions it has not resolved yet, a
//!   `<mentions>` block: each item's one-line summary from its provider's
//!   get function plus the pre-verified call for its full details (offered
//!   only when the session's policy allows that function). Both blocks are
//!   persisted by the harness and replayed, so the hook reads its own
//!   earlier blocks to never repeat itself.
//! - `judge::mentions::resolve` does the same resolution for any caller.

mod hook;
pub mod registry;
pub mod render;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use mention_contract::{format_mention, MentionRef, MentionView};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use hook::{plan, pre_generate, HookResponse, Plan, PreGenerateInput};
use registry::{Provider, ProviderRegistry};
use render::{DetailsCall, MentionStatus, ResolvedMention};

pub const PRE_GENERATE_FN: &str = "judge::mentions::pre-generate";
pub const RESOLVE_FN: &str = "judge::mentions::resolve";
/// Mentions resolved per generation; older unresolved ones wait.
pub const MAX_MENTIONS: usize = 10;
/// Size cap of one `<mentions>` block (about 600 tokens).
pub const BLOCK_BUDGET_CHARS: usize = 2_400;
/// One get call's budget. The binding's own timeout is 3 s.
const GET_TIMEOUT: Duration = Duration::from_millis(1_500);

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ResolveRequest {
    /// Text to find mentions in (markdown code is skipped).
    #[serde(default)]
    pub text: Option<String>,
    /// Mentions to resolve, in addition to those in `text`.
    #[serde(default)]
    pub mentions: Vec<MentionKey>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MentionKey {
    pub name: String,
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResolveResponse {
    pub mentions: Vec<ResolvedMention>,
}

/// Register both functions; the hook binding is separate (`bind`).
pub fn register(iii: &Arc<IIIClient>) {
    let registry = ProviderRegistry::default();

    let (client, cache) = (iii.clone(), registry.clone());
    iii.register_function(
        PRE_GENERATE_FN,
        RegisterFunction::new_async(move |input: PreGenerateInput| {
            let (client, cache) = (client.clone(), cache.clone());
            async move { Ok::<HookResponse, Error>(pre_generate(&client, &cache, input).await) }
        })
        .description(
            "Internal: harness pre-generate hook — tells agents which @<name>(id=…) mentions they may write and resolves the ones users wrote into summaries plus pre-verified details calls.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let (client, cache) = (iii.clone(), registry);
    iii.register_function(
        RESOLVE_FN,
        RegisterFunction::new_async(move |request: ResolveRequest| {
            let (client, cache) = (client.clone(), cache.clone());
            async move {
                let mut mentions = request
                    .text
                    .as_deref()
                    .map(mention_contract::parse_prose_mentions)
                    .unwrap_or_default();
                mentions.extend(request.mentions.into_iter().map(|key| MentionRef {
                    token: format_mention(&key.name, &key.id),
                    name: key.name,
                    id: key.id,
                }));
                let mut seen = HashSet::new();
                mentions.retain(|mention| seen.insert(mention.key()));
                mentions.truncate(MAX_MENTIONS * 5);
                let providers = cache.providers(&client).await;
                Ok::<ResolveResponse, Error>(ResolveResponse {
                    mentions: resolve(&client, &providers, &mentions, None).await,
                })
            }
        })
        .description(
            "Resolve @<name>(id=\"…\") chat mentions (found in `text`, or listed) through the workers that provide them: each one's status, one-line summary and the call that returns its full details.",
        ),
    );
}

/// Bind the hook. Best effort: without a harness (standalone judge) the
/// engine parks the binding until `harness::hook::pre-generate` exists.
/// `fail_open` is mandatory — pre-generate hooks fail closed by default,
/// and a mention that could not be resolved must never block a turn.
pub fn bind(iii: &IIIClient) {
    let binding = RegisterTriggerInput::new(
        "harness::hook::pre-generate".to_string(),
        PRE_GENERATE_FN.to_string(),
        json!({ "priority": 100, "timeout_ms": 3_000, "on_error": "fail_open" }),
    );
    match iii.register_trigger(binding) {
        Ok(_) => tracing::info!("mention resolution joins agent generations"),
        Err(error) => tracing::warn!(%error, "mention hook binding failed (harness absent?)"),
    }
}

/// Resolve each mention through its provider's get function, concurrently,
/// in input order. `allowed` (the session's callable functions) gates the
/// details call: `None` offers it always.
pub async fn resolve(
    iii: &Arc<IIIClient>,
    providers: &[Provider],
    mentions: &[MentionRef],
    allowed: Option<&HashSet<String>>,
) -> Vec<ResolvedMention> {
    let mut tasks = tokio::task::JoinSet::new();
    for (index, mention) in mentions.iter().enumerate() {
        let Some(provider) = providers
            .iter()
            .find(|p| p.descriptor.name == mention.name)
            .cloned()
        else {
            continue;
        };
        let (client, id) = (iii.clone(), mention.id.clone());
        tasks.spawn(async move {
            let reply = client
                .trigger(TriggerRequest {
                    function_id: provider.get_function_id.clone(),
                    payload: json!({ "id": id }),
                    action: None,
                    timeout_ms: Some(GET_TIMEOUT.as_millis() as u64),
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|value| {
                    serde_json::from_value::<Option<MentionView>>(value)
                        .map_err(|error| format!("unreadable view: {error}"))
                });
            (index, reply)
        });
    }
    let mut replies = std::collections::HashMap::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok((index, reply)) = joined {
            replies.insert(index, reply);
        }
    }
    mentions
        .iter()
        .enumerate()
        .map(|(index, mention)| {
            let provider = providers.iter().find(|p| p.descriptor.name == mention.name);
            outcome(mention, provider, replies.remove(&index), allowed)
        })
        .collect()
}

fn outcome(
    mention: &MentionRef,
    provider: Option<&Provider>,
    reply: Option<Result<Option<MentionView>, String>>,
    allowed: Option<&HashSet<String>>,
) -> ResolvedMention {
    let mut resolved = ResolvedMention {
        token: format_mention(&mention.name, &mention.id),
        name: mention.name.clone(),
        id: mention.id.clone(),
        status: MentionStatus::UnknownProvider,
        label: None,
        summary: None,
        details: None,
        error: None,
    };
    let Some(provider) = provider else {
        return resolved;
    };
    resolved.label = Some(provider.descriptor.label.clone());
    match reply {
        Some(Ok(Some(view))) => {
            resolved.status = MentionStatus::Resolved;
            resolved.summary = Some(view.agent_summary());
            resolved.details = provider
                .descriptor
                .details
                .as_ref()
                .filter(|details| allowed.is_none_or(|set| set.contains(&details.function_id)))
                .map(|details| DetailsCall {
                    function_id: details.function_id.clone(),
                    // The canonical id: a key the user typed resolves to the
                    // provider's own id.
                    payload: details.payload(&view.id),
                });
        }
        Some(Ok(None)) => resolved.status = MentionStatus::NotFound,
        Some(Err(error)) => {
            resolved.status = MentionStatus::Error;
            resolved.error = Some(error);
        }
        None => {
            resolved.status = MentionStatus::Error;
            resolved.error = Some("the provider did not answer".into());
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use mention_contract::MentionProvider;

    fn provider() -> Provider {
        Provider {
            get_function_id: "kanban::mention::get".into(),
            descriptor: MentionProvider::new("kanban", "Tickets", "kanban::mention::search")
                .details("kanban::ticket::get", "id"),
        }
    }

    fn mention(id: &str) -> MentionRef {
        MentionRef {
            name: "kanban".into(),
            id: id.into(),
            token: format_mention("kanban", id),
        }
    }

    fn view(id: &str) -> MentionView {
        serde_json::from_value(json!({
            "id": id,
            "label": "Fix login",
            "summary": "Kanban ticket KAN-1 \"Fix login\" · status: To do"
        }))
        .unwrap()
    }

    #[test]
    fn a_resolved_mention_offers_details_by_its_canonical_id() {
        let resolved = outcome(
            &mention("KAN-1"),
            Some(&provider()),
            Some(Ok(Some(view("uuid-1")))),
            None,
        );
        assert_eq!(resolved.status, MentionStatus::Resolved);
        assert_eq!(resolved.label.as_deref(), Some("Tickets"));
        assert_eq!(
            resolved.details,
            Some(DetailsCall {
                function_id: "kanban::ticket::get".into(),
                payload: json!({ "id": "uuid-1" }),
            })
        );
    }

    #[test]
    fn details_follow_the_session_policy() {
        let allowed: HashSet<String> = ["other::fn".to_string()].into();
        let denied = outcome(
            &mention("1"),
            Some(&provider()),
            Some(Ok(Some(view("1")))),
            Some(&allowed),
        );
        assert_eq!(denied.status, MentionStatus::Resolved);
        assert_eq!(denied.details, None);
    }

    #[test]
    fn unknown_ids_providers_and_failures_say_so() {
        assert_eq!(
            outcome(&mention("1"), Some(&provider()), Some(Ok(None)), None).status,
            MentionStatus::NotFound
        );
        assert_eq!(
            outcome(&mention("1"), None, None, None).status,
            MentionStatus::UnknownProvider
        );
        let failed = outcome(
            &mention("1"),
            Some(&provider()),
            Some(Err("timeout".into())),
            None,
        );
        assert_eq!(failed.status, MentionStatus::Error);
        assert_eq!(failed.error.as_deref(), Some("timeout"));
        let silent = outcome(&mention("1"), Some(&provider()), None, None);
        assert_eq!(silent.status, MentionStatus::Error);
    }
}
