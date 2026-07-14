//! Harness integration: the `pre-generate` injection hook and the
//! `turn-completed` extraction trigger.
//!
//! Injection contract (harness/src/hooks/runner.rs):
//! - blocks extend the SYSTEM PROMPT (stable per session → provider prompt
//!   cache stays warm across turns),
//! - recalled facts arrive as ONE APPENDED MESSAGE (they vary per turn;
//!   appending never invalidates the cached system-prompt prefix).
//!
//! Both handlers are registered `internal` and the harness binding is
//! `on_error: fail_open` — a memory failure must never block or fail a
//! turn. Handlers are idempotent: redelivered steps re-run them safely.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;

use crate::deps::Deps;
use crate::extract;
use crate::types::now_ms;

pub const PRE_GENERATE_FN: &str = "memory::hook::pre-generate";
pub const TURN_COMPLETED_FN: &str = "memory::on-turn-completed";
pub const EXTRACT_JOB_FN: &str = "memory::extract-job";
/// Durable queue (iii-queue builtin or the queue worker) carrying one
/// extraction job per completed turn: retries + DLQ instead of a lost
/// pass when this worker restarts mid-extraction.
pub const EXTRACTION_QUEUE: &str = "memory-extraction";

/// Envelope the harness posts to `pre-generate` hooks. Only the fields
/// this worker reads are typed; everything else is ignored.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateInput {
    #[serde(default)]
    pub session_id: String,
    /// Turn metadata (`harness::send` options.metadata); `memory_bank`
    /// here overrides the session-level selection for this turn.
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub generate: Option<GenerateInput>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateInput {
    #[serde(default)]
    pub system_prompt: String,
    /// Transcript messages as assembled so far (schema-free: shapes belong
    /// to the router).
    #[serde(default)]
    pub messages: Value,
}

/// Hook reply: `continue` with optional mutations (never deny — memory is
/// an enrichment, not a gate).
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub append_messages: Vec<Value>,
}

impl HookResponse {
    pub fn pass() -> Self {
        Self {
            decision: "continue".into(),
            mutations: None,
            annotations: None,
        }
    }
}

/// `harness::turn-completed` payload (the fields this worker reads).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TurnCompletedInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

/// One durable extraction job (the `data` of a queue message).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExtractJobInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AckResponse {
    pub ok: bool,
}

/// Inject the bank's blocks + recalled facts for one turn. Every failure
/// path degrades to a plain `continue` — never an error, never a
/// cross-bank fallback.
pub async fn pre_generate(
    deps: &Arc<Deps>,
    input: PreGenerateInput,
) -> Result<HookResponse, Error> {
    let cfg = deps.config().await;
    if !cfg.inject_blocks && !cfg.inject_facts {
        return Ok(HookResponse::pass());
    }
    if input.session_id.is_empty() {
        return Ok(HookResponse::pass());
    }

    // Turn metadata wins, then session metadata, then the default. A
    // session-lookup failure injects nothing (scope-safe).
    let turn_bank = input
        .metadata
        .as_ref()
        .and_then(|m| m.get("memory_bank"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let bank_name = match turn_bank {
        Some(b) => b,
        None => {
            match extract::resolve_bank(&deps.iii, &input.session_id, &cfg.default_bank).await {
                Some(b) => b,
                None => return Ok(HookResponse::pass()),
            }
        }
    };
    let store = deps.store().await;
    let Ok(bank) = store.bank(&bank_name).await else {
        // Unknown bank = nothing stored yet; extraction creates it later.
        return Ok(HookResponse::pass());
    };

    let mut mutations = Mutations::default();
    let mut annotations = Map::new();
    annotations.insert("memory_bank".into(), json!(bank_name));

    if cfg.inject_blocks {
        if let Ok(blocks) = bank.list_blocks() {
            if !blocks.is_empty() {
                let generate = input.generate.as_ref();
                let base = generate.map(|g| g.system_prompt.as_str()).unwrap_or("");
                let mut section = format!("\n\n# Memory — bank: {bank_name}\n");
                for b in &blocks {
                    section.push_str(&format!("\n## {}\n{}\n", b.name, b.content.trim()));
                }
                mutations.system_prompt = Some(format!("{base}{section}"));
                annotations.insert("memory_blocks".into(), json!(blocks.len()));
            }
        }
    }

    if cfg.inject_facts {
        let query = last_user_text(input.generate.as_ref().map(|g| &g.messages));
        if !query.trim().is_empty() {
            let hits = bank
                .recall(&query, cfg.recall_limit, cfg.decay_half_life_days, false)
                .await;
            let mut budget = (cfg.recall_budget_tokens * 4) as usize;
            let mut lines = Vec::new();
            for (fact, _) in &hits {
                if fact.text.len() > budget {
                    break;
                }
                budget -= fact.text.len();
                lines.push(format!("- {}", fact.text));
            }
            if !lines.is_empty() {
                let body = format!(
                    "<memory bank=\"{bank_name}\">\nRelevant remembered facts (auto-recalled; verify anything surprising):\n{}\n</memory>",
                    lines.join("\n")
                );
                mutations.append_messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": body }],
                    "timestamp": now_ms() as i64,
                }));
                annotations.insert("memory_facts".into(), json!(lines.len()));
            }
        }
    }

    if mutations.system_prompt.is_none() && mutations.append_messages.is_empty() {
        return Ok(HookResponse::pass());
    }
    Ok(HookResponse {
        decision: "continue".into(),
        mutations: Some(mutations),
        annotations: Some(annotations),
    })
}

/// Ack fast; hand the extraction pass to the durable queue (retries +
/// DLQ, receipt-id deduped by turn), falling back to an inline spawn when
/// no queue surface is installed. At-least-once redelivery is safe either
/// way: fingerprints make the whole pass idempotent.
pub async fn turn_completed(
    deps: &Arc<Deps>,
    input: TurnCompletedInput,
) -> Result<AckResponse, Error> {
    let cfg = deps.config().await;
    let completed = input.status.as_deref().is_none_or(|s| s == "completed");
    if cfg.extraction_enabled && completed && !input.session_id.is_empty() {
        let receipt = input
            .turn_id
            .clone()
            .unwrap_or_else(|| format!("t{}", now_ms()));
        let enqueued = deps
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::queue::enqueue".into(),
                payload: json!({
                    "queue": EXTRACTION_QUEUE,
                    "function_id": EXTRACT_JOB_FN,
                    "data": { "session_id": input.session_id },
                    "messageReceiptId": format!("memx-{receipt}"),
                }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;
        if let Err(e) = enqueued {
            tracing::debug!(error = %e, "queue surface unavailable; extracting inline");
            let deps = deps.clone();
            let session_id = input.session_id;
            tokio::spawn(async move { extract::run(deps, session_id).await });
        }
    }
    Ok(AckResponse { ok: true })
}

/// Queue-delivered extraction job. Returns the error to the queue so a
/// failed pass retries and eventually lands in the DLQ instead of
/// vanishing.
pub async fn extract_job(deps: &Arc<Deps>, input: ExtractJobInput) -> Result<AckResponse, Error> {
    extract::try_run(deps, &input.session_id)
        .await
        .map_err(Error::Handler)?;
    Ok(AckResponse { ok: true })
}

/// The newest user message's text — the recall query.
fn last_user_text(messages: Option<&Value>) -> String {
    let Some(items) = messages.and_then(Value::as_array) else {
        return String::new();
    };
    for m in items.iter().rev() {
        if m.get("role").and_then(Value::as_str) == Some("user") {
            let text = crate::extract::content_text(m.get("content"));
            // Skip our own injected wrapper when the hook chain re-runs.
            if !text.trim_start().starts_with("<memory ") {
                return text;
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_user_text_takes_newest_and_skips_injected() {
        let messages = json!([
            { "role": "user", "content": [{ "type": "text", "text": "older question" }] },
            { "role": "assistant", "content": [{ "type": "text", "text": "answer" }] },
            { "role": "user", "content": [{ "type": "text", "text": "newest question" }] },
            { "role": "user", "content": [{ "type": "text", "text": "<memory bank=\"m\">…</memory>" }] },
        ]);
        assert_eq!(last_user_text(Some(&messages)), "newest question");
        assert_eq!(last_user_text(None), "");
    }

    #[test]
    fn hook_response_serializes_wire_shape() {
        let resp = HookResponse {
            decision: "continue".into(),
            mutations: Some(Mutations {
                system_prompt: Some("sp".into()),
                append_messages: vec![json!({ "role": "user" })],
            }),
            annotations: None,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["decision"], "continue");
        assert_eq!(v["mutations"]["system_prompt"], "sp");
        assert!(v["mutations"]["append_messages"].is_array());
    }
}
