//! Harness integration: the `pre-generate` injection hook and the
//! `turn-completed` extraction trigger.
//!
//! Injection contract (harness/src/hooks/runner.rs):
//! - rules extend the SYSTEM PROMPT (stable per session → provider prompt
//!   cache stays warm across turns),
//! - recalled memories arrive as ONE APPENDED MESSAGE (they vary per turn;
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
use crate::types::{now_ms, Memory};

pub const PRE_GENERATE_FN: &str = "memory::hook::pre-generate";
pub const TURN_COMPLETED_FN: &str = "memory::on-turn-completed";
pub const EXTRACT_JOB_FN: &str = "memory::extract-job";
pub const SESSION_DELETED_FN: &str = "memory::on-session-deleted";
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

/// Inject the bank's rules + recalled memories for one turn. Every failure
/// path degrades to a plain `continue` — never an error, never a
/// cross-bank fallback.
pub async fn pre_generate(
    deps: &Arc<Deps>,
    input: PreGenerateInput,
) -> Result<HookResponse, Error> {
    let cfg = deps.config().await;
    if !cfg.inject_rules && !cfg.inject_memories {
        return Ok(HookResponse::pass());
    }
    if input.session_id.is_empty() {
        return Ok(HookResponse::pass());
    }

    // Turn metadata wins, then session metadata, then the default. A
    // session-lookup failure injects nothing (scope-safe).
    let bank_name = match select_turn_bank(input.metadata.as_ref()) {
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

    // Always present (stable per session, so it never breaks the provider
    // prompt cache): without this, models apologize that they "can't save
    // to memory" when a user asks them to remember something — capture is
    // ambient and needs no function call.
    let generate = input.generate.as_ref();
    let base = generate.map(|g| g.system_prompt.as_str()).unwrap_or("");
    let mut section = format!(
        "\n\n# Memory — bank: {bank_name}\nMemory is automatic here: durable memories and \
         preferences from this conversation are extracted and saved after each turn, and \
         relevant memories are provided to you when they exist. When the user asks you to \
         remember something, acknowledge it — no save call is needed.\n"
    );

    if cfg.inject_rules {
        if let Ok(rules) = bank.list_rules() {
            if !rules.is_empty() {
                let (rules_part, truncated) = build_rules_section(&rules, cfg.max_rule_chars);
                section.push_str(&rules_part);
                if truncated {
                    tracing::warn!(
                        bank = %bank_name,
                        max_rule_chars = cfg.max_rule_chars,
                        "memory rules exceed the injection budget; truncated"
                    );
                    annotations.insert("memory_rules_truncated".into(), json!(true));
                }
                annotations.insert("memory_rules".into(), json!(rules.len()));
            }
        }
    }
    mutations.system_prompt = Some(format!("{base}{section}"));

    if cfg.inject_memories {
        let query = last_user_text(input.generate.as_ref().map(|g| &g.messages));
        if !query.trim().is_empty() {
            // Fail-soft semantic signal; lexical-only when it misses.
            let query_vec = crate::embed_client::query_vector(deps, &query).await;
            if query_vec.is_some() {
                annotations.insert("memory_retrieval".into(), json!("bm25-entity-semantic"));
            }
            let hits = bank
                .recall(
                    &query,
                    query_vec.as_deref(),
                    cfg.recall_limit,
                    cfg.decay_half_life_days,
                    false,
                )
                .await;
            // Ambient floor: lexical recall misses identity questions and
            // session openers entirely (their words never appear in memory
            // texts), so pad thin results with the bank's strongest memories
            // — still bounded by the same count and token budget.
            let hits: Vec<Memory> = hits.into_iter().map(|(f, _)| f).collect();
            let top = bank.top_memories(cfg.recall_limit).await;
            let memories = apply_ambient_floor(hits, top, cfg.recall_limit);
            if let Some((body, ids)) =
                memories_message(&bank_name, &memories, cfg.recall_budget_tokens)
            {
                let count = ids.len();
                mutations.append_messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": body }],
                    "timestamp": now_ms() as i64,
                }));
                annotations.insert("memory_recalled".into(), json!(count));
                // Ids (not texts) so chat surfaces can render "which memories
                // fed this turn" chips and fetch details on demand.
                annotations.insert("memory_ids".into(), json!(ids));
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

/// `session::deleted` payload (the field this worker reads).
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SessionDeletedInput {
    #[serde(default)]
    pub session_id: String,
}

/// GC the per-session extraction cursor. Memories keep their provenance ids
/// (history), but the operational cursor has nothing left to point at.
pub async fn session_deleted(
    deps: &Arc<Deps>,
    input: SessionDeletedInput,
) -> Result<AckResponse, Error> {
    if !input.session_id.is_empty() {
        extract::cursor_delete(&deps.iii, &input.session_id).await;
    }
    Ok(AckResponse { ok: true })
}

/// Turn-scoped bank override: `memory_bank` in the turn metadata.
pub(crate) fn select_turn_bank(metadata: Option<&Value>) -> Option<String> {
    metadata
        .and_then(|m| m.get("memory_bank"))
        .and_then(Value::as_str)
        .filter(|b| !b.is_empty())
        .map(str::to_string)
}

/// The rules part of the injected system-prompt section, hard-bounded by
/// `max_rule_chars`. Over-budget content truncates on a char boundary
/// with a visible marker when at least 200 chars of budget remain, and is
/// omitted (named) otherwise. Returns `(section_part, truncated)`.
pub(crate) fn build_rules_section(
    rules: &[crate::types::Rule],
    max_rule_chars: usize,
) -> (String, bool) {
    let mut out = String::new();
    let mut budget = max_rule_chars;
    let mut truncated = false;
    for rule in rules {
        let content = rule.content.trim();
        if content.len() <= budget {
            budget -= content.len();
            out.push_str(&format!("\n## {}\n{content}\n", rule.name));
        } else if budget > 200 {
            let mut cut = budget.min(content.len());
            while cut > 0 && !content.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push_str(&format!(
                "\n## {}\n{}\n[rule truncated: over the {} char injection budget — trim it in the memory page]\n",
                rule.name,
                &content[..cut],
                max_rule_chars,
            ));
            budget = 0;
            truncated = true;
        } else {
            out.push_str(&format!(
                "\n## {} [omitted: over the injection budget]\n",
                rule.name
            ));
            truncated = true;
        }
    }
    (out, truncated)
}

/// Pad thin recall results with the bank's strongest memories, deduped by
/// id and bounded by `min(AMBIENT_FLOOR, recall_limit)`.
pub(crate) fn apply_ambient_floor(
    mut memories: Vec<Memory>,
    top: Vec<Memory>,
    recall_limit: usize,
) -> Vec<Memory> {
    const AMBIENT_FLOOR: usize = 3;
    let floor = AMBIENT_FLOOR.min(recall_limit);
    if memories.len() < floor {
        for memory in top {
            if memories.len() >= floor {
                break;
            }
            if !memories.iter().any(|f| f.id == memory.id) {
                memories.push(memory);
            }
        }
    }
    memories
}

/// The one appended `<memory>` message: bulleted memory texts under a
/// 4-chars-per-token budget, plus the ids of the memories that made the
/// cut. `None` when nothing fits.
pub(crate) fn memories_message(
    bank: &str,
    memories: &[Memory],
    recall_budget_tokens: u64,
) -> Option<(String, Vec<String>)> {
    let mut budget = (recall_budget_tokens * 4) as usize;
    let mut lines = Vec::new();
    let mut ids = Vec::new();
    for memory in memories {
        if memory.text.len() > budget {
            break;
        }
        budget -= memory.text.len();
        lines.push(format!("- {}", memory.text));
        ids.push(memory.id.clone());
    }
    if lines.is_empty() {
        return None;
    }
    let body = format!(
        "<memory bank=\"{bank}\">\nRelevant remembered memories (auto-recalled; verify anything surprising):\n{}\n</memory>",
        lines.join("\n")
    );
    Some((body, ids))
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

    fn rule(name: &str, content: &str) -> crate::types::Rule {
        crate::types::Rule {
            name: name.into(),
            content: content.into(),
            updated_at: 0,
        }
    }

    fn memory(id: &str, text: &str) -> Memory {
        Memory {
            id: id.into(),
            text: text.into(),
            entities: vec![],
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: 1,
            updated_at: 1,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        }
    }

    #[test]
    fn turn_bank_override_wins_and_empty_is_none() {
        assert_eq!(
            select_turn_bank(Some(&json!({ "memory_bank": "blog" }))),
            Some("blog".to_string())
        );
        assert_eq!(select_turn_bank(Some(&json!({ "memory_bank": "" }))), None);
        assert_eq!(select_turn_bank(Some(&json!({ "memory_bank": 7 }))), None);
        assert_eq!(select_turn_bank(Some(&json!({ "other": "x" }))), None);
        assert_eq!(select_turn_bank(None), None);
    }

    #[test]
    fn rules_within_budget_inject_whole_and_unmarked() {
        let rules = vec![rule("style", "No em-dashes."), rule("tone", "Formal.")];
        let (section, truncated) = build_rules_section(&rules, 6_000);
        assert!(!truncated);
        assert!(section.contains(
            "## style
No em-dashes."
        ));
        assert!(section.contains(
            "## tone
Formal."
        ));
        assert!(!section.contains("truncated"));
    }

    #[test]
    fn over_budget_rule_truncates_with_visible_marker() {
        let big = "x".repeat(1_000);
        let (section, truncated) = build_rules_section(&[rule("big", &big)], 300);
        assert!(truncated);
        assert!(section.contains("[rule truncated: over the 300 char injection budget"));
        // The kept prefix respects the budget.
        assert!(section.contains(&"x".repeat(300)));
        assert!(!section.contains(&"x".repeat(301)));
    }

    #[test]
    fn truncation_cuts_on_a_char_boundary() {
        // é is two bytes; an odd budget must not split it.
        let content = "é".repeat(400);
        let (section, truncated) = build_rules_section(&[rule("uni", &content)], 301);
        assert!(truncated);
        assert!(section.contains("[rule truncated"));
    }

    #[test]
    fn tiny_leftover_budget_omits_by_name_instead_of_truncating() {
        let rules = vec![
            rule("first", &"a".repeat(900)),
            rule("second", &"b".repeat(500)),
        ];
        // First consumes 900 of 1000; 100 left is under the 200-char floor.
        let (section, truncated) = build_rules_section(&rules, 1_000);
        assert!(truncated);
        assert!(section.contains("## second [omitted: over the injection budget]"));
        assert!(!section.contains("bbbb"));
    }

    #[test]
    fn ambient_floor_pads_thin_results_and_dedups() {
        let hits = vec![memory("fp1", "hit")];
        let top = vec![
            memory("fp1", "hit"),
            memory("fp2", "strong"),
            memory("fp3", "also strong"),
            memory("fp4", "never reached"),
        ];
        let out = apply_ambient_floor(hits, top, 6);
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["fp1", "fp2", "fp3"]);
    }

    #[test]
    fn ambient_floor_respects_a_small_recall_limit() {
        let out = apply_ambient_floor(vec![], vec![memory("fp1", "a"), memory("fp2", "b")], 1);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn ambient_floor_leaves_rich_results_alone() {
        let hits = vec![memory("fp1", "a"), memory("fp2", "b"), memory("fp3", "c")];
        let out = apply_ambient_floor(hits.clone(), vec![memory("fp9", "top")], 6);
        assert_eq!(out.len(), 3);
        assert!(!out.iter().any(|m| m.id == "fp9"));
    }

    #[test]
    fn memories_message_formats_body_and_ids_in_lockstep() {
        let ms = vec![memory("fp1", "first"), memory("fp2", "second")];
        let (body, ids) = memories_message("blog", &ms, 1_200).unwrap();
        assert!(body.starts_with("<memory bank=\"blog\">"));
        assert!(body.contains("- first\n- second"));
        assert!(body.ends_with("</memory>"));
        assert_eq!(ids, vec!["fp1", "fp2"]);
    }

    #[test]
    fn memories_message_stops_at_the_token_budget() {
        // Budget of 1 token = 4 chars: the 5-char text does not fit.
        let ms = vec![memory("fp1", "12345"), memory("fp2", "abc")];
        let out = memories_message("b", &ms, 1);
        // First memory over budget stops the loop entirely (ordered list,
        // no skipping ahead) — nothing fits.
        assert!(out.is_none());
    }

    #[test]
    fn memories_message_budget_boundary_is_inclusive() {
        let ms = vec![memory("fp1", "1234")];
        let (_, ids) = memories_message("b", &ms, 1).unwrap();
        assert_eq!(ids, vec!["fp1"]);
    }

    #[test]
    fn pre_generate_input_tolerates_the_harness_envelope() {
        // Captured shape of a real harness pre-generate call: extra fields
        // must be ignored, not rejected.
        let raw = json!({
            "session_id": "s_123",
            "turn_id": "t_9",
            "hook_point": "pre-generate",
            "metadata": { "memory_bank": "blog", "other": true },
            "generate": {
                "system_prompt": "You are helpful.",
                "messages": [
                    { "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }
                ],
                "model": "claude-sonnet-5"
            }
        });
        let input: PreGenerateInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.session_id, "s_123");
        assert_eq!(
            select_turn_bank(input.metadata.as_ref()),
            Some("blog".to_string())
        );
        let generate = input.generate.unwrap();
        assert_eq!(generate.system_prompt, "You are helpful.");
        assert_eq!(last_user_text(Some(&generate.messages)), "hi");
    }

    #[test]
    fn turn_completed_input_defaults_status_to_completed_semantics() {
        let input: TurnCompletedInput =
            serde_json::from_value(json!({ "session_id": "s_1", "turn_id": "t_1" })).unwrap();
        assert!(input.status.as_deref().is_none_or(|s| s == "completed"));
        let failed: TurnCompletedInput =
            serde_json::from_value(json!({ "session_id": "s_1", "status": "failed" })).unwrap();
        assert!(failed.status.as_deref().is_some_and(|s| s != "completed"));
    }

    #[test]
    fn pass_response_carries_no_mutations() {
        let v = serde_json::to_value(HookResponse::pass()).unwrap();
        assert_eq!(v["decision"], "continue");
        assert!(v.get("mutations").is_none());
        assert!(v.get("annotations").is_none());
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
