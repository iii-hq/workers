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

use crate::clients::RouterClient;
use crate::error::HarnessError;
use crate::types::event::Usage;
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

/// One generation's context accounting. `total <= usable` always holds for
/// a generation that ran; `free = usable - total`.
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

pub async fn delete(
    iii: &IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    crate::state::state_delete(iii, CONTEXT_SCOPE, session_id, timeout_ms).await
}

/// Process-lifetime cache of provider token counts keyed by
/// (model, kind, content hash) — the system prompt and tool schemas are
/// stable across a session's steps, so each is counted over the wire once.
fn count_cache() -> &'static Mutex<HashMap<u64, u64>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, u64>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(model: &str, kind: &str, content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    model.hash(&mut hasher);
    kind.hash(&mut hasher);
    content.hash(&mut hasher);
    hasher.finish()
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

async fn counted_delta(
    router: &RouterClient,
    model: &str,
    provider: Option<&str>,
    kind: &str,
    content_key: &str,
    system_prompt: Option<&str>,
    tools: Option<&[AgentFunction]>,
) -> Option<(u64, String)> {
    let key = cache_key(model, kind, content_key);
    if let Some(tokens) = count_cache().lock().ok()?.get(&key).copied() {
        return Some((tokens, "provider".into()));
    }
    let probe = probe_messages();
    let (base, _) = router
        .count_tokens(model, provider, None, None, &probe)
        .await?;
    let (with_part, estimator) = router
        .count_tokens(model, provider, system_prompt, tools, &probe)
        .await?;
    let tokens = with_part.saturating_sub(base);
    if let Ok(mut cache) = count_cache().lock() {
        cache.insert(key, tokens);
    }
    Some((tokens, estimator))
}

/// Replace the snapshot's estimated categories with provider-exact numbers
/// where they exist: the generation's billed usage is the exact total, the
/// system prompt and tool schemas are counted once via the provider's
/// tokenizer (cached by content), and the message window is the remainder.
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
    let model = snapshot.model.clone();

    let system_exact = match system_prompt {
        Some(sp) if !sp.is_empty() => {
            counted_delta(
                &router.clone(),
                &model,
                provider.as_deref(),
                "system_prompt",
                sp,
                Some(sp),
                None,
            )
            .await
        }
        _ => Some((0, String::new())),
    };
    let tools_exact = if tools.is_empty() {
        Some((0, String::new()))
    } else {
        let tools_key = serde_json::to_string(tools).unwrap_or_default();
        counted_delta(
            &router.clone(),
            &model,
            provider.as_deref(),
            "tools",
            &tools_key,
            None,
            Some(tools),
        )
        .await
    };
    let (Some((system_tokens, sys_est)), Some((tools_tokens, tools_est))) =
        (system_exact, tools_exact)
    else {
        return;
    };
    let estimator = [sys_est, tools_est]
        .into_iter()
        .find(|e| !e.is_empty())
        .unwrap_or_else(|| "provider".into());

    let remainder = billed
        .saturating_sub(system_tokens)
        .saturating_sub(tools_tokens);
    let heuristic_messages = {
        let m = &snapshot.categories.messages;
        m.user + m.assistant + m.function_result + m.custom
    };
    // Keep the by-role proportions from the estimate but rescale them onto
    // the exact remainder (providers report only the request total).
    let scaled = if heuristic_messages > 0 {
        let scale = remainder as f64 / heuristic_messages as f64;
        let m = &snapshot.categories.messages;
        SnapshotMessagesV1 {
            user: (m.user as f64 * scale) as u64,
            assistant: (m.assistant as f64 * scale) as u64,
            function_result: (m.function_result as f64 * scale) as u64,
            custom: (m.custom as f64 * scale) as u64,
        }
    } else {
        SnapshotMessagesV1 {
            user: remainder,
            assistant: 0,
            function_result: 0,
            custom: 0,
        }
    };

    snapshot.categories.system_prompt = system_tokens;
    snapshot.categories.tools = tools_tokens;
    snapshot.categories.messages = scaled;
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
            timestamp: 1_722_700_000_000,
        };
        let mut value = serde_json::to_value(&snapshot).unwrap();
        value["from_the_future"] = serde_json::json!(true);
        let parsed: ContextSnapshotV1 = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, snapshot);
    }
}
