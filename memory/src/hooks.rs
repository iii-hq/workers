//! Harness integration: the `pre-generate` injection hook and the
//! `turn-completed` extraction trigger.
//!
//! Injection contract (harness/src/hooks/runner.rs). The prefix a model
//! reasoned under must never change mid-session (append-only), and every
//! appended message is persisted into the transcript and replayed:
//! - rules extend the SYSTEM PROMPT once, rendered on the session's first
//!   step and then sent verbatim; a later change (rules edited, a bank
//!   switch, memory turned off) arrives as ONE appended update message,
//!   never as a system-prompt edit,
//! - recalled memories arrive as ONE APPENDED MESSAGE on a turn's first
//!   step only (the persisted copy carries them through later steps).
//!
//! Both handlers are registered `internal` and the harness binding is
//! `on_error: fail_open` — a memory failure must never block or fail a
//! turn. Handlers are idempotent: redelivered steps re-run them safely.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

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
    /// Step within the turn; 0 is the step that opens it.
    #[serde(default)]
    pub step: u64,
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
    /// Transcript messages as assembled so far — the window this step
    /// sends, persisted notices (earlier rules updates included) replayed
    /// in place (schema-free: shapes belong to the router).
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
    if input.session_id.is_empty() {
        return Ok(HookResponse::pass());
    }
    let mut mutations = Mutations::default();
    let mut annotations = Map::new();
    if !cfg.inject_rules && !cfg.inject_memories {
        // Memory off is an empty section, told like any rules change: a
        // section this session already froze stays in its system prompt.
        apply_rules(&input, "", &mut mutations, &mut annotations);
        return Ok(HookResponse {
            decision: "continue".into(),
            mutations: Some(mutations),
            annotations: Some(annotations),
        });
    }

    // Turn metadata wins, then session metadata, then the default. A
    // session-lookup failure injects nothing new (scope-safe), but keeps
    // the section this session's system prompt already carries.
    let bank_name = match select_turn_bank(input.metadata.as_ref()) {
        Some(b) => b,
        None => {
            match extract::resolve_bank(&deps.iii, &input.session_id, &cfg.default_bank).await {
                Some(b) => b,
                None => return Ok(keep_told_section(&input)),
            }
        }
    };
    let bank = deps.store().await.bank(&bank_name).await.ok();
    annotations.insert("memory_bank".into(), json!(bank_name));

    // The ambient header: without it, models apologize that they
    // "can't save to memory" when a user asks them to remember something —
    // capture is ambient and needs no function call. An unknown bank has
    // nothing stored yet: with extraction on the header still goes in (the
    // section does not change once extraction creates the bank); with it
    // off nothing ever will, and the header's promise would be false.
    let mut section = if bank.is_some() || cfg.extraction_enabled {
        ambient_header(&bank_name)
    } else {
        String::new()
    };

    if let Some(bank) = bank.as_ref().filter(|_| cfg.inject_rules) {
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
    apply_rules(&input, &section, &mut mutations, &mut annotations);

    // Recall only on the step that opens the turn: the appended message is
    // persisted and replayed on later steps, so re-recalling would stack copies.
    if let Some(bank) = bank
        .as_ref()
        .filter(|_| cfg.inject_memories && input.step == 0)
    {
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
                mutations.append_messages.push(user_text_message(body));
                annotations.insert("memory_recalled".into(), json!(count));
                // Ids (not texts) so chat surfaces can render "which memories
                // fed this turn" chips and fetch details on demand.
                annotations.insert("memory_ids".into(), json!(ids));
            }
        }
    }

    Ok(HookResponse {
        decision: "continue".into(),
        mutations: Some(mutations),
        annotations: Some(annotations),
    })
}

/// A plain user-role text message for `append_messages`.
fn user_text_message(text: String) -> Value {
    json!({
        "role": "user",
        "content": [{ "type": "text", "text": text }],
        "timestamp": now_ms() as i64,
    })
}

/// Sessions tracked in [`TOLD_RULES`]; the least recently used is evicted
/// past this.
const TOLD_RULES_CAP: usize = 4_096;

/// The memory section each session's system prompt froze on its first step.
// ponytail: in-process only — a worker restart forgets it, so the next step
// re-renders the CURRENT section into the system prompt (one mid-session
// prompt edit for sessions whose rules changed); persist per session in
// state if that matters.
static TOLD_RULES: LazyLock<Mutex<HashMap<String, ToldRules>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct ToldRules {
    /// The section first rendered into this session's system prompt (empty
    /// when memory injected nothing then); sent verbatim on every later step.
    system: String,
    used_ms: u64,
}

/// Opens the appended rules update. Starts like the recall message
/// (`<memory `), so recall queries and extraction skip it.
const RULES_UPDATE_OPEN: &str = "<memory update=\"rules\">";

/// Append-only rules delivery for one step: the session's frozen section
/// into the system prompt (left untouched while that section is empty) and,
/// when `section` is not what the model last saw, one update message.
fn apply_rules(
    input: &PreGenerateInput,
    section: &str,
    mutations: &mut Mutations,
    annotations: &mut Map<String, Value>,
) {
    let generate = input.generate.as_ref();
    let (frozen, update) = {
        let mut told = TOLD_RULES.lock().unwrap_or_else(|e| e.into_inner());
        tell_rules(
            &mut told,
            &input.session_id,
            generate.map(|g| &g.messages),
            section,
            now_ms(),
        )
    };
    if !frozen.is_empty() {
        let base = generate.map(|g| g.system_prompt.as_str()).unwrap_or("");
        mutations.system_prompt = Some(format!("{base}{frozen}"));
    }
    if let Some(update) = update {
        mutations.append_messages.push(user_text_message(update));
        annotations.insert("memory_rules_updated".into(), json!(true));
    }
}

/// The session's frozen section (`section` itself on its first step) and
/// the update to append when `section` differs from what the model last
/// saw: the newest rules update in `messages` (the window this step sends),
/// else the frozen section. Read from the transcript, not from what this
/// process sent, so it is idempotent: an update lost on the way (fail-open
/// timeout, a later hook's deny, a crash before it persisted) or compacted
/// away is sent again, and one the window holds never is.
fn tell_rules(
    told: &mut HashMap<String, ToldRules>,
    session_id: &str,
    messages: Option<&Value>,
    section: &str,
    now_ms: u64,
) -> (String, Option<String>) {
    let frozen = match told.get_mut(session_id) {
        Some(t) => {
            t.used_ms = now_ms;
            t.system.clone()
        }
        None => {
            if told.len() >= TOLD_RULES_CAP {
                if let Some(oldest) = told
                    .iter()
                    .min_by_key(|(_, t)| t.used_ms)
                    .map(|(id, _)| id.clone())
                {
                    told.remove(&oldest);
                }
            }
            told.insert(
                session_id.to_string(),
                ToldRules {
                    system: section.to_string(),
                    used_ms: now_ms,
                },
            );
            section.to_string()
        }
    };
    let update = rules_update_message(section);
    let seen = match latest_rules_update(messages) {
        Some(sent) => sent == update,
        None => frozen == section,
    };
    (frozen, (!seen).then_some(update))
}

/// The newest rules update in `messages`, verbatim.
fn latest_rules_update(messages: Option<&Value>) -> Option<String> {
    messages?
        .as_array()?
        .iter()
        .rev()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("user"))
        .map(|m| crate::extract::content_text(m.get("content")))
        .find(|text| text.trim_start().starts_with(RULES_UPDATE_OPEN))
}

/// The appended rules update: the whole current section, or a note that
/// none applies (memory off, or an unknown bank with extraction off).
fn rules_update_message(section: &str) -> String {
    let section = if section.is_empty() {
        "\nNo memory section applies now.\n"
    } else {
        section
    };
    format!(
        "{RULES_UPDATE_OPEN}\nThe memory rules changed. This block replaces the memory \
         section of the system prompt and any earlier update:{section}</memory>"
    )
}

/// Degraded path: inject nothing new, but keep the system-prompt section
/// the session already has so its prefix does not change. A pending rules
/// update waits for the next step that resolves the bank.
fn keep_told_section(input: &PreGenerateInput) -> HookResponse {
    let told = TOLD_RULES.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = told.get(&input.session_id).filter(|t| !t.system.is_empty()) else {
        return HookResponse::pass();
    };
    let base = input
        .generate
        .as_ref()
        .map(|g| g.system_prompt.as_str())
        .unwrap_or("");
    HookResponse {
        decision: "continue".into(),
        mutations: Some(Mutations {
            system_prompt: Some(format!("{base}{}", t.system)),
            append_messages: Vec::new(),
        }),
        annotations: None,
    }
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
        TOLD_RULES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&input.session_id);
    }
    Ok(AckResponse { ok: true })
}

/// The always-present opening of the injected section: tells the
/// model capture is ambient, so "remember X" gets an acknowledgment
/// instead of a false "I cannot save to memory".
pub(crate) fn ambient_header(bank_name: &str) -> String {
    format!(
        "\n\n# Memory — bank: {bank_name}\nMemory is automatic here: durable memories and \
         preferences from this conversation are extracted and saved after each turn, and \
         relevant memories are provided to you when they exist. When the user asks you to \
         remember something, acknowledge it — no save call is needed.\n"
    )
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
            tags: vec![],
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

    /// The window a step sends: plain user text messages.
    fn window(texts: &[&str]) -> Value {
        Value::Array(
            texts
                .iter()
                .map(|t| user_text_message(t.to_string()))
                .collect(),
        )
    }

    #[test]
    fn unchanged_rules_keep_the_system_prompt_and_append_nothing() {
        let mut told = HashMap::new();
        let section = "\n\n# Memory\n## a\nA\n";
        let (first, _) = tell_rules(&mut told, "s1", None, section, 1);
        let w = window(&["question"]);
        let (system, update) = tell_rules(&mut told, "s1", Some(&w), section, 2);
        assert_eq!(system, first);
        assert!(update.is_none());
    }

    #[test]
    fn added_rule_appends_one_update_until_the_window_holds_it() {
        let mut told = HashMap::new();
        let before = "\n\n# Memory\n## a\nA\n";
        let after = "\n\n# Memory\n## a\nA\n\n## b\nB\n";
        let (first, _) = tell_rules(&mut told, "s1", None, before, 1);

        let w = window(&["question"]);
        let (system, update) = tell_rules(&mut told, "s1", Some(&w), after, 2);
        assert_eq!(system, first, "system prompt must stay append-only");
        let update = update.expect("rule change appends an update");
        assert!(
            update.contains("## b\nB"),
            "update carries the full current rules"
        );
        assert!(update.contains("## a\nA"));
        // Starts like the recall message, so recall queries and extraction
        // skip it.
        assert!(update.starts_with("<memory "));
        assert_eq!(
            last_user_text(Some(&window(&["question", &update]))),
            "question"
        );

        // Not delivered (re-assembly retry, fail-open timeout, a later
        // hook's deny, a crash before it persisted): the window still lacks
        // it, so it is sent again.
        let (system, again) = tell_rules(&mut told, "s1", Some(&w), after, 3);
        assert_eq!(system, first);
        assert_eq!(again.as_deref(), Some(update.as_str()));

        // Delivered: the persisted copy replays in the window.
        let delivered = window(&["question", &update, "answer", "next"]);
        let (system, none) = tell_rules(&mut told, "s1", Some(&delivered), after, 4);
        assert_eq!(system, first);
        assert!(none.is_none(), "an update is sent once, not every step");

        // Compacted away: the window no longer carries it, so it is sent
        // again (the system prompt still holds the old rules).
        let compacted = window(&["summary of the earlier turns", "next"]);
        let (_, resent) = tell_rules(&mut told, "s1", Some(&compacted), after, 5);
        assert_eq!(resent.as_deref(), Some(update.as_str()));

        // Another session starts fresh with the current rules.
        let (other, update) = tell_rules(&mut told, "s2", None, after, 6);
        assert_eq!(other, after);
        assert!(update.is_none());
    }

    #[test]
    fn rules_changing_back_send_an_update() {
        let mut told = HashMap::new();
        let a = "\n\n# Memory\n## a\nA\n";
        let b = "\n\n# Memory\n## b\nB\n";
        tell_rules(&mut told, "s1", None, a, 1);
        let to_b = tell_rules(&mut told, "s1", None, b, 2).1.unwrap();
        let w = window(&["q1", &to_b, "q2"]);
        let (system, update) = tell_rules(&mut told, "s1", Some(&w), a, 3);
        assert_eq!(system, a);
        let to_a = update.expect("the model last saw B, so A is news again");
        assert!(to_a.contains("## a\nA") && !to_a.contains("## b"));
        let w = window(&["q1", &to_b, "q2", &to_a, "q3"]);
        assert!(tell_rules(&mut told, "s1", Some(&w), a, 4).1.is_none());
        // With no update left in the window, A is the frozen section.
        assert!(tell_rules(&mut told, "s1", Some(&window(&["q4"])), a, 5)
            .1
            .is_none());
    }

    #[test]
    fn memory_off_keeps_the_frozen_section_and_says_so_once() {
        let mut told = HashMap::new();
        let a = "\n\n# Memory\n## a\nA\n";
        tell_rules(&mut told, "s1", None, a, 1);
        let (system, off) = tell_rules(&mut told, "s1", None, "", 2);
        assert_eq!(system, a, "turning memory off is not a system-prompt edit");
        let off = off.expect("the model is told memory no longer applies");
        assert!(off.contains("No memory section applies now."));
        let w = window(&["q", &off]);
        assert!(tell_rules(&mut told, "s1", Some(&w), "", 3).1.is_none());
        // Back on: the rules arrive as an update, the prefix still A.
        let (system, on) = tell_rules(&mut told, "s1", Some(&w), a, 4);
        assert_eq!(system, a);
        assert!(on.is_some_and(|u| u.contains("## a\nA")));

        // A session that started with nothing freezes the empty section.
        let (system, update) = tell_rules(&mut told, "s2", None, "", 5);
        assert_eq!(system, "");
        assert!(update.is_none());
        let (system, update) = tell_rules(&mut told, "s2", None, a, 6);
        assert_eq!(system, "", "a later section is appended, not rendered");
        assert!(update.is_some_and(|u| u.contains("## a\nA")));
    }

    #[test]
    fn an_empty_frozen_section_leaves_the_system_prompt_alone() {
        let session = "s-empty-frozen";
        let input: PreGenerateInput = serde_json::from_value(json!({
            "session_id": session, "generate": { "system_prompt": "base" },
        }))
        .unwrap();
        let (mut mutations, mut annotations) = (Mutations::default(), Map::new());
        apply_rules(&input, "", &mut mutations, &mut annotations);
        assert!(mutations.system_prompt.is_none());
        assert!(mutations.append_messages.is_empty());
        assert!(annotations.is_empty());
        assert!(keep_told_section(&input).mutations.is_none());
        TOLD_RULES.lock().unwrap().remove(session);
    }

    #[test]
    fn degraded_step_keeps_the_frozen_section_and_appends_nothing() {
        let session = "s-degraded-keep";
        let (a, b) = ("\n\n# Memory\n## a\nA\n", "\n\n# Memory\n## b\nB\n");
        {
            let mut told = TOLD_RULES.lock().unwrap();
            tell_rules(&mut told, session, None, a, 1);
            tell_rules(&mut told, session, None, b, 2);
        }
        let input: PreGenerateInput = serde_json::from_value(json!({
            "session_id": session, "step": 1,
            "generate": { "system_prompt": "base" },
        }))
        .unwrap();
        let kept = keep_told_section(&input).mutations.unwrap();
        assert_eq!(
            kept.system_prompt.as_deref(),
            Some(format!("base{a}").as_str())
        );
        assert!(kept.append_messages.is_empty());
        TOLD_RULES.lock().unwrap().remove(session);
    }

    #[test]
    fn told_rules_evict_the_least_recently_used_session_at_the_cap() {
        let mut told = HashMap::new();
        for i in 0..TOLD_RULES_CAP as u64 {
            tell_rules(&mut told, &format!("s{i}"), None, "x", i + 10);
        }
        tell_rules(&mut told, "s0", None, "x", 1_000_000); // refresh s0
        tell_rules(&mut told, "new", None, "x", 1_000_001);
        assert_eq!(told.len(), TOLD_RULES_CAP);
        assert!(told.contains_key("s0"));
        assert!(!told.contains_key("s1"), "least recently used goes first");
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
