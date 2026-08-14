//! Per-generation context snapshot (`harness_context/<session_id>`): what
//! the last generation's model window held, by category, plus the provider
//! usage that came back for it. Built from the assembly the loop already
//! performs (no extra counting round trips), stamped with usage after the
//! terminal frame, stored once per generate step. Read back by
//! `harness::metrics` and pushed on `harness::turn-completed`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::sync::OnceLock;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::clients::{LoadedEntry, RouterClient};
use crate::error::HarnessError;
use crate::types::event::Usage;
use crate::types::message::AgentMessage;
use crate::types::model::AgentFunction;

pub const CONTEXT_SCOPE: &str = "harness_context";

/// Estimated tokens of the assembled window's messages, by role.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotMessagesV1 {
    pub user: u64,
    pub assistant: u64,
    pub function_result: u64,
    pub custom: u64,
}

impl SnapshotMessagesV1 {
    /// What the whole messages category contributes to the request total.
    pub fn total(&self) -> u64 {
        self.user + self.assistant + self.function_result + self.custom
    }

    /// The same by-role proportions carried onto a different total
    /// (`numerator / denominator`), summing to exactly `numerator`: the
    /// largest bucket absorbs the flooring remainder so category totals
    /// keep reconciling with the exact request total. `denominator == 0`
    /// has no proportions to carry, so it yields all-zero.
    fn scaled(&self, numerator: u64, denominator: u64) -> Self {
        if denominator == 0 {
            return Self::default();
        }
        let scale = numerator as f64 / denominator as f64;
        let apply = |tokens: u64| (tokens as f64 * scale) as u64;
        let mut scaled = Self {
            user: apply(self.user),
            assistant: apply(self.assistant),
            function_result: apply(self.function_result),
            custom: apply(self.custom),
        };
        let shortfall = numerator.saturating_sub(scaled.total());
        let largest = [
            (&mut scaled.user, self.user),
            (&mut scaled.assistant, self.assistant),
            (&mut scaled.function_result, self.function_result),
            (&mut scaled.custom, self.custom),
        ]
        .into_iter()
        .max_by_key(|(_, original)| *original)
        .map(|(bucket, _)| bucket);
        if let Some(bucket) = largest {
            *bucket += shortfall;
        }
        scaled
    }
}

impl From<crate::clients::context::ByRoleTokens> for SnapshotMessagesV1 {
    fn from(by_role: crate::clients::context::ByRoleTokens) -> Self {
        Self {
            user: by_role.user,
            assistant: by_role.assistant,
            function_result: by_role.function_result,
            custom: by_role.custom,
        }
    }
}

/// Where the request's tokens sit. Categories are assembly-time estimates;
/// `hook_guidance` is the measured growth after assembly (pre-generate hook
/// appends and orphan-repair patches), 0 when the request left assembly
/// unchanged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotCategoriesV1 {
    /// Final assembled system prompt: mode paragraph, identity, per-step
    /// aids, and any compaction summary section.
    pub system_prompt: u64,
    /// Function schemas exposed to the model.
    pub tools: u64,
    pub messages: SnapshotMessagesV1,
    /// Provider framing plus response_format / provider_options fields.
    pub overhead: u64,
    #[serde(default)]
    pub hook_guidance: u64,
}

/// One generation's context accounting. `free = usable - total`, floored at
/// zero: once provider usage lands, `total` is what was billed, which can
/// exceed the `usable` budget the window was fit into — that budget was
/// derived before the generation from an estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextSnapshotV1 {
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Which estimator produced the numbers (`heuristic` until the
    /// context-manager resolves a real tokenizer). Absent when the
    /// context-manager predates the breakdown response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator: Option<String>,
    /// The input budget the window was fit into.
    pub usable: u64,
    /// Output allocation `usable` was derived against.
    pub effective_max_output_tokens: u64,
    /// Final request estimate: categories plus post-assembly growth.
    pub total: u64,
    pub free: u64,
    pub categories: SnapshotCategoriesV1,
    pub compacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarized_head_tokens: Option<u64>,
    /// Actual provider usage for this generation, stamped after the
    /// terminal frame; absent when the provider returned none (or the
    /// generation never completed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Running cost of the whole session in USD, accumulated across every
    /// generation step. Absent when any completed step did not report a cost:
    /// a partial sum must not be presented as the whole session's bill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_cost_usd: Option<f64>,
    pub timestamp: i64,
}

/// Store the session's latest snapshot (whole-value write; the loop holds
/// the only writer per session).
pub async fn put(
    iii: &IIIClient,
    snapshot: &ContextSnapshotV1,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let value = serde_json::to_value(snapshot)
        .map_err(|e| HarnessError::State(format!("context snapshot serialize: {e}")))?;
    crate::state::state_set(iii, CONTEXT_SCOPE, &snapshot.session_id, value, timeout_ms).await
}

/// The session's latest snapshot (`None` when absent or unparseable — a
/// snapshot from a newer harness must degrade to "no data", never an error).
pub async fn get(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<Option<ContextSnapshotV1>, HarnessError> {
    let v = crate::state::state_get(iii, CONTEXT_SCOPE, session_id, timeout_ms).await?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(v).ok())
}

/// Drop a session's snapshot row. Called when the session itself is deleted
/// (`harness::on-session-deleted`), so the state worker does not keep a row
/// for a session nobody can read any more.
pub async fn delete(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    crate::state::state_delete(iii, CONTEXT_SCOPE, session_id, timeout_ms).await
}

/// Sum every prior assistant generation on the active transcript plus the
/// current generation. The deterministic current entry is skipped because a
/// redelivered step may already have appended or updated it. Any missing price
/// makes the whole value unknown, matching `harness::metrics` completeness.
pub(crate) fn session_cost_from_entries(
    entries: &[LoadedEntry],
    current_assistant_id: &str,
    step_cost_usd: Option<f64>,
) -> Option<f64> {
    let previous = entries.iter().try_fold(0.0, |total, entry| {
        if entry.entry_id == current_assistant_id {
            return Some(total);
        }
        let Some(AgentMessage::Assistant(assistant)) = &entry.message else {
            return Some(total);
        };
        assistant
            .usage
            .as_ref()
            .and_then(|usage| usage.cost_usd)
            .map(|cost| total + cost)
    })?;
    step_cost_usd.map(|cost| previous + cost)
}

/// A cached count and the estimator that answered it. Caching the estimator
/// is what keeps the provenance honest: without it every step after the first
/// would report its counts as coming from the provider's own default rather
/// than from the tokenizer that actually produced them.
type CachedCount = (u64, Option<String>);

/// Process-lifetime cache of provider token counts keyed by
/// (model, kind, content hash) — the probe base, the system prompt and the
/// tool schemas are stable across a session's steps, so each is counted over
/// the wire once.
fn count_cache() -> &'static Mutex<HashMap<u64, CachedCount>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, CachedCount>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(model: &str, provider: Option<&str>, kind: &str, content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    provider.hash(&mut hasher);
    kind.hash(&mut hasher);
    content.hash(&mut hasher);
    hasher.finish()
}

/// Cache miss and a poisoned lock are the same thing to the caller: count it
/// over the wire again.
fn cached(key: u64) -> Option<CachedCount> {
    count_cache().lock().ok()?.get(&key).cloned()
}

fn cache(key: u64, tokens: u64, estimator: Option<String>) {
    if let Ok(mut cache) = count_cache().lock() {
        cache.insert(key, (tokens, estimator));
    }
}

/// One-message probe the delta counts subtract away: provider metering
/// endpoints refuse an empty messages array, so category counts are
/// measured as count(probe + part) - count(probe).
fn probe_messages() -> Vec<serde_json::Value> {
    vec![json!({
        "role": "user",
        "content": [{ "type": "text", "text": "x" }],
        "timestamp": 0
    })]
}

/// The request part being counted. Carries everything the count needs: which
/// `count_tokens` field it fills, its cache kind, and its cache content key.
enum Part<'a> {
    System(&'a str),
    Tools(&'a [AgentFunction]),
}

impl Part<'_> {
    fn kind(&self) -> &'static str {
        match self {
            Part::System(_) => "system_prompt",
            Part::Tools(_) => "tools",
        }
    }

    fn content_key(&self) -> String {
        match self {
            Part::System(prompt) => (*prompt).to_string(),
            Part::Tools(tools) => serde_json::to_string(tools).unwrap_or_default(),
        }
    }
}

/// One part's tokenizer-derived token count.
#[derive(Debug, Default)]
struct Counted {
    tokens: u64,
    /// Which estimator answered this count, over the wire or from the cache.
    /// `None` only for a part the request did not carry.
    estimator: Option<String>,
}

/// Tokens the probe alone costs, so every part's delta subtracts the same
/// base. Counted once per (model, provider) and cached for the process.
async fn probe_base(router: &RouterClient, model: &str, provider: Option<&str>) -> Option<u64> {
    let key = cache_key(model, provider, "base", "");
    if let Some((tokens, _)) = cached(key) {
        return Some(tokens);
    }
    // The base is subtracted away from every part, so which estimator counted
    // it never reaches the snapshot; only the parts' estimators do.
    let (base, _) = router
        .count_tokens(model, provider, None, None, &probe_messages())
        .await?;
    cache(key, base, None);
    Some(base)
}

/// Tokenizer-derived tokens for one part: count(probe + part) - `base`, cached
/// by content. `None` means the provider count failed and the caller must
/// keep its heuristic numbers.
async fn counted_delta(
    router: &RouterClient,
    model: &str,
    provider: Option<&str>,
    base: u64,
    part: Part<'_>,
) -> Option<Counted> {
    let key = cache_key(model, provider, part.kind(), &part.content_key());
    if let Some((tokens, estimator)) = cached(key) {
        return Some(Counted { tokens, estimator });
    }
    let (system_prompt, tools) = match part {
        Part::System(prompt) => (Some(prompt), None),
        Part::Tools(tools) => (None, Some(tools)),
    };
    let (with_part, estimator) = router
        .count_tokens(model, provider, system_prompt, tools, &probe_messages())
        .await?;
    let tokens = with_part.saturating_sub(base);
    cache(key, tokens, Some(estimator.clone()));
    Some(Counted {
        tokens,
        estimator: Some(estimator),
    })
}

/// Reconcile the snapshot against provider usage where it exists: the
/// generation's billed usage supplies the prompt total, the system prompt and
/// tool schemas are counted once via the configured tokenizer (cached by
/// content), and the message window is the remainder.
/// A rig without a provider counter keeps the heuristic snapshot untouched.
pub async fn exactify(
    snapshot: &mut ContextSnapshotV1,
    router: &RouterClient,
    system_prompt: Option<&str>,
    tools: &[AgentFunction],
) {
    let Some(usage) = snapshot.usage.as_ref() else {
        return;
    };
    let billed =
        usage.input.unwrap_or(0) + usage.cache_read.unwrap_or(0) + usage.cache_write.unwrap_or(0);
    if billed == 0 {
        return;
    }
    let provider = snapshot.provider.clone();
    let provider = provider.as_deref();
    let model = snapshot.model.clone();

    // Every part's delta subtracts the same probe base, so count it once for
    // the whole snapshot rather than once per part.
    let Some(base) = probe_base(router, &model, provider).await else {
        return;
    };

    let system_part = system_prompt.filter(|p| !p.is_empty()).map(Part::System);
    let tools_part = (!tools.is_empty()).then_some(Part::Tools(tools));
    // Independent round trips: run them together.
    let (counted_system, counted_tools) = tokio::join!(
        async {
            match system_part {
                Some(part) => counted_delta(router, &model, provider, base, part).await,
                None => Some(Counted::default()),
            }
        },
        async {
            match tools_part {
                Some(part) => counted_delta(router, &model, provider, base, part).await,
                None => Some(Counted::default()),
            }
        },
    );
    let (Some(counted_system), Some(counted_tools)) = (counted_system, counted_tools) else {
        return;
    };
    let estimator = counted_system
        .estimator
        .or(counted_tools.estimator)
        .unwrap_or_else(|| "provider".into());

    let remainder = billed
        .saturating_sub(counted_system.tokens)
        .saturating_sub(counted_tools.tokens);
    let heuristic_messages = snapshot.categories.messages.total();
    // Keep the by-role proportions from the estimate but rescale them onto
    // the exact remainder (providers report only the request total).
    let messages = if heuristic_messages > 0 {
        snapshot
            .categories
            .messages
            .scaled(remainder, heuristic_messages)
    } else {
        SnapshotMessagesV1 {
            user: remainder,
            ..SnapshotMessagesV1::default()
        }
    };

    snapshot.categories.system_prompt = counted_system.tokens;
    snapshot.categories.tools = counted_tools.tokens;
    snapshot.categories.messages = messages;
    snapshot.categories.overhead = 0;
    snapshot.categories.hook_guidance = 0;
    snapshot.total = billed;
    snapshot.free = snapshot.usable.saturating_sub(billed);
    snapshot.estimator = Some(estimator);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trips_and_tolerates_unknown_fields() {
        let snapshot = ContextSnapshotV1 {
            session_id: "s_1".into(),
            turn_id: "t_1".into(),
            step: 3,
            model: "m".into(),
            provider: Some("p".into()),
            estimator: Some("heuristic".into()),
            usable: 100_000,
            effective_max_output_tokens: 8_192,
            total: 62_000,
            free: 38_000,
            categories: SnapshotCategoriesV1 {
                system_prompt: 2_400,
                tools: 9_800,
                messages: SnapshotMessagesV1 {
                    user: 10_000,
                    assistant: 20_000,
                    function_result: 19_000,
                    custom: 0,
                },
                overhead: 300,
                hook_guidance: 500,
            },
            compacted: true,
            summarized_head_tokens: Some(3_100),
            usage: Some(Usage {
                input: Some(61_400),
                output: Some(900),
                cache_read: Some(55_000),
                cache_write: None,
                reasoning: None,
                cost_usd: Some(0.42),
            }),
            session_cost_usd: Some(1.37),
            timestamp: 1_722_700_000_000,
        };
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["from_the_future"] = serde_json::json!(true);
        let parsed: ContextSnapshotV1 = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn session_cost_requires_every_assistant_generation_to_be_known() {
        let mut assistant = crate::types::message::empty_assistant("p", "m");
        assistant.usage = Some(Usage {
            cost_usd: Some(1.37),
            ..Usage::default()
        });
        let mut entries = vec![LoadedEntry {
            entry_id: "assistant_1".into(),
            message: Some(AgentMessage::Assistant(assistant)),
            custom: None,
        }];

        assert_eq!(
            session_cost_from_entries(&[], "assistant_current", Some(0.42)),
            Some(0.42)
        );
        assert_eq!(
            session_cost_from_entries(&entries, "assistant_current", Some(0.42)),
            Some(1.79)
        );
        assert_eq!(
            session_cost_from_entries(&entries, "assistant_current", None),
            None
        );

        let mut current = crate::types::message::empty_assistant("p", "m");
        current.usage = Some(Usage {
            cost_usd: Some(99.0),
            ..Usage::default()
        });
        entries.push(LoadedEntry {
            entry_id: "assistant_current".into(),
            message: Some(AgentMessage::Assistant(current)),
            custom: None,
        });
        assert_eq!(
            session_cost_from_entries(&entries, "assistant_current", Some(0.42)),
            Some(1.79)
        );

        let Some(AgentMessage::Assistant(assistant)) = &mut entries[0].message else {
            unreachable!()
        };
        assistant.usage.as_mut().unwrap().cost_usd = None;
        assert_eq!(
            session_cost_from_entries(&entries, "assistant_current", Some(0.42)),
            None
        );
    }
}
