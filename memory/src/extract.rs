//! Post-turn memory extraction: one `router::complete` call per completed
//! turn, off the hot path (spawned from the `harness::turn-completed`
//! handler, never inside a turn).
//!
//! ADD-only by design: extraction never rewrites or deletes existing memories.
//! The content fingerprint makes redelivery and re-observation idempotent —
//! a known memory is reinforced (`corroboration += 1`), never duplicated.
//! Memory's own function calls never reach this path: the transcript is
//! fetched with `roles: [user, assistant]`, and only text parts are read.
//!
//! INCREMENTAL: a per-session cursor (last processed entry id, kept in the
//! `state` worker under scope `memory_cursor`) means each pass reads and
//! extracts only the messages that arrived since the previous pass — the
//! LLM never re-pays for content it already saw. The cursor advances only
//! after a successful pass, so a failed call retries over the same span.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::events::ItemEvent;
use crate::store::CommitKind;
use crate::types::{fingerprint, now_ms, Confidence, Memory, Provenance};

const EXTRACT_SYSTEM: &str = "You extract durable memories from a conversation excerpt.\n\
Return ONLY a JSON array (no prose, no code fences). Each element:\n\
{\"text\": string, \"entities\": [string], \"confidence\": \"extracted\"|\"inferred\", \
\"kind\": \"memory\"|\"rule\"}\n\
Rules:\n\
- Only durable content worth remembering across sessions: stable preferences, corrections, \
identities, project constants, standing instructions.\n\
- Never ephemeral state (current task progress, one-off values), never secrets, API keys, \
tokens, or passwords.\n\
- text: one self-contained sentence, max 200 characters, in the language of the source.\n\
- entities: short lowercase handles for the people/projects/tools the memory is about.\n\
- confidence: \"extracted\" when stated directly, \"inferred\" when derived.\n\
- kind \"rule\" ONLY for standing instructions about how the assistant should behave going \
forward — style directives, formatting conventions, workflow corrections (\"never use \
em-dashes\", \"always write tests first\"). Facts about the user, projects, or the world are \
kind \"memory\". When unsure, use \"memory\".\n\
- Return [] when nothing qualifies. Most turns have nothing worth keeping.";

/// Name of the auto-managed rule that captured standing instructions
/// append to. Hand-authored rules are never touched by extraction.
const LEARNED_RULE: &str = "learned";
const LEARNED_HEADER: &str = "# Learned\nStanding instructions captured automatically from \
conversations. Edit or delete lines freely — extraction only ever appends new ones.\n";
/// Cap on rule promotions accepted from a single extraction pass.
const MAX_RULES_PER_TURN: usize = 3;

#[derive(Debug, Deserialize)]
struct ExtractedItem {
    text: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}

impl ExtractedItem {
    fn is_rule(&self) -> bool {
        self.kind.as_deref() == Some("rule")
    }
}

/// Append new standing instructions to the learned rule, skipping any whose
/// content fingerprint already appears (re-observation is a no-op). Returns
/// the merged content and how many lines were actually added.
fn merge_learned(existing: &str, additions: &[String]) -> (String, usize) {
    let mut seen: std::collections::HashSet<String> = existing
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .map(fingerprint)
        .collect();
    let fresh: Vec<&String> = additions
        .iter()
        .filter(|t| seen.insert(fingerprint(t)))
        .collect();
    if fresh.is_empty() {
        return (existing.to_string(), 0);
    }
    let mut out = if existing.trim().is_empty() {
        LEARNED_HEADER.to_string()
    } else {
        existing.trim_end().to_string() + "\n"
    };
    for t in &fresh {
        out.push_str("- ");
        out.push_str(t);
        out.push('\n');
    }
    (out, fresh.len())
}

/// Resolve the bank for a session: session metadata `memory_bank` wins,
/// then the configured default. A metadata FETCH FAILURE resolves to
/// `None` — injecting or writing the default bank when the session's real
/// bank is unknown would leak across contexts.
pub async fn resolve_bank(iii: &IIIClient, session_id: &str, default_bank: &str) -> Option<String> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "session::get".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    match reply {
        Ok(v) => {
            let from_meta = v
                .pointer("/meta/metadata/memory_bank")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(from_meta.unwrap_or_else(|| default_bank.to_string()))
        }
        Err(e) => {
            tracing::warn!(session_id, error = %e, "session::get failed; skipping memory for this turn (scope-safe)");
            None
        }
    }
}

/// Run one extraction pass for a completed turn. Errors are logged, never
/// propagated — extraction is a background enrichment, not a turn
/// dependency.
pub async fn run(deps: Arc<Deps>, session_id: String) {
    if let Err(e) = try_run(&deps, &session_id).await {
        tracing::warn!(session_id = %session_id, error = %e, "extraction pass failed");
    }
}

pub async fn try_run(deps: &Deps, session_id: &str) -> Result<(), String> {
    let cfg = deps.config().await;
    let Some(bank_name) = resolve_bank(&deps.iii, session_id, &cfg.default_bank).await else {
        return Ok(());
    };

    let since = cursor_get(&deps.iii, session_id).await;
    let (transcript, newest_id) = fetch_transcript_since(
        &deps.iii,
        session_id,
        since.as_deref(),
        cfg.extraction_window,
    )
    .await?;
    if transcript.trim().len() < 20 {
        // Nothing new worth an LLM call; still advance past trivia so the
        // same span is never re-fetched.
        if let Some(id) = newest_id {
            cursor_set(&deps.iii, session_id, &id).await;
        }
        return Ok(());
    }

    let model = resolve_model(&deps.iii, &cfg.extraction_model).await?;
    let reply = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "router::complete".into(),
            payload: json!({
                "model": model,
                "system_prompt": EXTRACT_SYSTEM,
                "messages": [{
                    "role": "user",
                    "content": [{ "type": "text", "text": transcript }],
                    "timestamp": now_ms() as i64,
                }],
                "max_output_tokens": 1024,
            }),
            action: None,
            timeout_ms: Some(cfg.extraction_timeout_ms),
        })
        .await
        .map_err(|e| format!("router::complete: {e}"))?;

    let text = assistant_text(&reply);
    let items = parse_items(&text)?;
    if items.is_empty() {
        // Nothing to write: the span is done, advance past it.
        if let Some(id) = newest_id {
            cursor_set(&deps.iii, session_id, &id).await;
        }
        return Ok(());
    }

    let store = deps.store().await;
    let (bank, _) = store
        .ensure_bank(&bank_name, None)
        .await
        .map_err(|e| e.to_string())?;

    // Standing instructions graduate straight into the always-injected
    // `learned` rule (visible + editable in the rules tab) instead of the
    // recall pool — Mike's loop: correct in chat, review the rules page.
    let (rules, items): (Vec<ExtractedItem>, Vec<ExtractedItem>) =
        items.into_iter().partition(ExtractedItem::is_rule);
    if cfg.rule_learning_enabled && !rules.is_empty() {
        let additions: Vec<String> = rules
            .into_iter()
            .take(MAX_RULES_PER_TURN)
            .map(|r| r.text.trim().to_string())
            .filter(|t| t.len() >= 3 && t.len() <= 500)
            .collect();
        let existing = bank
            .list_rules()
            .ok()
            .and_then(|rs| rs.into_iter().find(|r| r.name == LEARNED_RULE))
            .map(|r| r.content)
            .unwrap_or_default();
        let (merged, added) = merge_learned(&existing, &additions);
        if added > 0 {
            match bank.set_rule(LEARNED_RULE, &merged) {
                Ok(()) => {
                    deps.emitter.bank("rules-changed", &bank_name).await;
                    tracing::info!(session_id, bank = %bank_name, added, "extraction pass learned rules");
                }
                Err(e) => tracing::warn!(error = %e, "learned rule write failed"),
            }
        }
    }

    let mut saved = 0usize;
    for item in items.into_iter().take(cfg.max_memories_per_turn) {
        let text = item.text.trim();
        if text.len() < 3 || text.len() > 500 {
            continue;
        }
        let id = fingerprint(text);
        let now = now_ms();
        let (memory, event) = match bank.get(&id).await {
            // Known memory re-observed: reinforce, never duplicate. A pinned
            // or tombstoned record is left exactly as it is.
            Some(existing) if existing.is_live() && !existing.pinned => {
                let mut f = existing;
                f.corroboration = f.corroboration.saturating_add(1);
                f.updated_at = now;
                f.revision += 1;
                (f, ItemEvent::Updated)
            }
            Some(_) => continue,
            None => (
                Memory {
                    id,
                    text: text.to_string(),
                    entities: item
                        .entities
                        .into_iter()
                        .map(|e| e.trim().to_lowercase())
                        .filter(|e| !e.is_empty())
                        .take(8)
                        .collect(),
                    confidence: match item.confidence.as_deref() {
                        Some("inferred") => Confidence::Inferred,
                        _ => Confidence::Extracted,
                    },
                    corroboration: 0,
                    pinned: false,
                    source: Some(Provenance {
                        session_id: Some(session_id.to_string()),
                        entry_id: None,
                        agent: None,
                    }),
                    created_at: now,
                    updated_at: now,
                    invalid_at: None,
                    superseded_by: None,
                    revision: 0,
                },
                ItemEvent::Created,
            ),
        };
        match bank.commit(memory.clone()).await {
            Ok(CommitKind::Created | CommitKind::Updated) => {
                deps.emitter.item(event, &bank_name, &memory).await;
                saved += 1;
            }
            // Propagate: the cursor must not advance past a span whose
            // writes did not all land. The queue retries the pass;
            // fingerprints make the retry reinforce, not duplicate.
            Err(e) => return Err(format!("memory commit failed: {e}")),
        }
    }
    // Every durable write landed: NOW the span is processed.
    if let Some(id) = newest_id {
        cursor_set(&deps.iii, session_id, &id).await;
    }
    if saved > 0 {
        tracing::info!(session_id, bank = %bank_name, saved, "extraction pass saved memories");
        // Embed the new memories off this path; recall degrades gracefully
        // until the vectors land.
        let deps_bg = crate::deps::Deps {
            iii: deps.iii.clone(),
            store: deps.store.clone(),
            config: deps.config.clone(),
            emitter: deps.emitter.clone(),
        };
        tokio::spawn(crate::embed_client::backfill_bank(
            Arc::new(deps_bg),
            bank_name.clone(),
        ));
    }
    Ok(())
}

const CURSOR_SCOPE: &str = "memory_cursor";
/// Page size while walking a session. The walk follows `next_cursor` to
/// the leaf (a hard page cap would permanently strand sessions longer
/// than the cap: `session::messages` is root->leaf, so the new turn lives
/// on the LAST page); RAM stays bounded because only the window tail and
/// the span after the stored cursor are retained. MAX_PAGES only bounds a
/// pathological/looping cursor, sized far above any real session.
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 10_000;

/// Last processed entry id for a session, from the state worker. Any
/// failure (state absent, no cursor yet) means "no cursor" — the pass
/// falls back to the newest `window` messages.
async fn cursor_get(iii: &IIIClient, session_id: &str) -> Option<String> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": CURSOR_SCOPE, "key": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok()?;
    // state::get returns the stored value at the top level; older engines
    // wrapped it under `value` — accept both.
    reply
        .pointer("/last_entry_id")
        .or_else(|| reply.pointer("/value/last_entry_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Best-effort cursor write; a miss only costs a re-read next pass
/// (fingerprints keep re-extraction harmless, just not free).
async fn cursor_set(iii: &IIIClient, session_id: &str, entry_id: &str) {
    let res = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": CURSOR_SCOPE,
                "key": session_id,
                "value": { "last_entry_id": entry_id },
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    if let Err(e) = res {
        tracing::debug!(session_id, error = %e, "cursor write failed (state worker absent?)");
    }
}

/// Drop a session's cursor (bound to `session::deleted`).
pub async fn cursor_delete(iii: &IIIClient, session_id: &str) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "state::delete".into(),
            payload: json!({ "scope": CURSOR_SCOPE, "key": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
}

/// User/assistant messages NEWER than `since` (all of them, capped to the
/// most recent `window`), plus the newest entry id seen. Walks the
/// session page by page — `session::messages` returns root→leaf, so the
/// new turn lives on the LAST page. `roles` narrowing also excludes
/// custom + function_result entries, so memory's own calls never feed
/// back into extraction.
async fn fetch_transcript_since(
    iii: &IIIClient,
    session_id: &str,
    since: Option<&str>,
    window: usize,
) -> Result<(String, Option<String>), String> {
    let mut collected: Vec<(String, String, String)> = Vec::new();
    let mut since_seen = false;
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut payload = json!({
            "session_id": session_id,
            "limit": PAGE_SIZE,
            "roles": ["user", "assistant"],
        });
        if let Some(c) = &cursor {
            payload["cursor"] = json!(c);
        }
        let reply = iii
            .trigger(TriggerRequest {
                function_id: "session::messages".into(),
                payload,
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
            .map_err(|e| format!("session::messages: {e}"))?;
        if let Some(items) = reply.get("messages").and_then(Value::as_array) {
            for item in items {
                let entry_id = item
                    .get("entry_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let Some(message) = item.get("message") else {
                    continue;
                };
                let role = message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let text = content_text(message.get("content"));
                collected.push((entry_id, role, text));
            }
        }
        // Bound RAM on long sessions: everything at or before the stored
        // cursor is already extracted, so once the cursor id streams past
        // we only need what follows it; without a cursor only the window
        // tail is ever read.
        if let Some(s) = since {
            if let Some(pos) = collected.iter().position(|(id, _, _)| id == s) {
                since_seen = true;
                collected.drain(..=pos);
            } else if !since_seen && collected.len() > window {
                let excess = collected.len() - window;
                collected.drain(..excess);
            }
        } else if collected.len() > window {
            let excess = collected.len() - window;
            collected.drain(..excess);
        }
        match reply.get("next_cursor").and_then(Value::as_str) {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }

    let newest_id = collected.last().map(|(id, _, _)| id.clone());

    // `collected` now holds entries strictly after the stored cursor (or
    // the tail when the cursor id was never seen: branch switch, trimmed
    // history); cap the LLM excerpt to the newest `window` either way.
    let start = collected.len().saturating_sub(window);

    let mut lines = Vec::new();
    for (_, role, text) in collected.into_iter().skip(start).rev().take(window) {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() || trimmed.starts_with("<memory ") {
            continue;
        }
        let mut excerpt = trimmed;
        excerpt.truncate(clamp_char_boundary(&excerpt, 2_000));
        lines.push(format!("{role}: {excerpt}"));
    }
    lines.reverse();
    Ok((lines.join("\n"), newest_id))
}

/// Largest index <= `max` that lands on a UTF-8 char boundary of `s`.
pub(crate) fn clamp_char_boundary(s: &str, max: usize) -> usize {
    let mut cut = max.min(s.len());
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Concatenated text parts of one message's content array.
pub fn content_text(content: Option<&Value>) -> String {
    let Some(parts) = content.and_then(Value::as_array) else {
        // Tolerate plain-string content from non-harness agents.
        return content.and_then(Value::as_str).unwrap_or("").to_string();
    };
    parts
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The extraction model: the configured pin, else the first catalog entry
/// (logged so operators notice the implicit choice).
async fn resolve_model(iii: &IIIClient, configured: &str) -> Result<String, String> {
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "router::models::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| format!("router::models::list: {e}"))?;
    let first = reply
        .get("models")
        .and_then(Value::as_array)
        .and_then(|m| m.first())
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match first {
        Some(id) => {
            tracing::info!(model = %id, "extraction_model not configured; using first router model");
            Ok(id)
        }
        None => Err("router catalog is empty; set extraction_model or add a provider".into()),
    }
}

fn assistant_text(reply: &Value) -> String {
    content_text(reply.pointer("/message/content"))
}

/// Parse the model reply. Tolerates code fences and stray prose around the
/// array, rejects everything else.
fn parse_items(raw: &str) -> Result<Vec<ExtractedItem>, String> {
    let trimmed = raw.trim();
    let candidate = match (trimmed.find('['), trimmed.rfind(']')) {
        (Some(start), Some(end)) if end > start => &trimmed[start..=end],
        _ => return Ok(Vec::new()),
    };
    serde_json::from_str::<Vec<ExtractedItem>>(candidate)
        .map_err(|e| format!("unparseable extraction reply: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tolerates_fences_and_prose() {
        let raw = "Here you go:\n```json\n[{\"text\": \"user prefers formal writing\", \"entities\": [\"mike\"]}]\n```";
        let items = parse_items(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "user prefers formal writing");
    }

    #[test]
    fn parse_empty_and_no_array_are_empty() {
        assert!(parse_items("[]").unwrap().is_empty());
        assert!(parse_items("nothing worth keeping").unwrap().is_empty());
    }

    #[test]
    fn clamp_never_splits_a_code_point() {
        let s = format!("{}é", "a".repeat(1_999));
        let cut = clamp_char_boundary(&s, 2_000);
        assert_eq!(cut, 1_999, "the two-byte é at 1999..2001 must not split");
        assert!(s.is_char_boundary(cut));
        assert_eq!(clamp_char_boundary("short", 2_000), 5);
        assert_eq!(clamp_char_boundary("", 10), 0);
    }

    #[test]
    fn parse_unknown_kind_is_a_memory_not_a_rule() {
        let raw = r#"[{"text": "x y z", "kind": "directive"}]"#;
        let items = parse_items(raw).unwrap();
        assert_eq!(items.len(), 1);
        assert!(!items[0].is_rule());
    }

    #[test]
    fn parse_rejects_non_array_json_objects() {
        assert!(parse_items(r#"{"text": "not a list"}"#).unwrap().is_empty());
    }

    #[test]
    fn merge_learned_never_touches_existing_lines() {
        let existing = "# Learned\nintro\n- Keep me exactly.\n";
        let (merged, added) = merge_learned(existing, &["New line.".into()]);
        assert_eq!(added, 1);
        assert!(merged.contains("- Keep me exactly.\n"));
        assert!(merged.contains("intro"));
        assert!(merged.ends_with("- New line.\n"));
    }

    #[test]
    fn parse_reads_rule_kind() {
        let raw = r#"[
            {"text": "never use em-dashes", "kind": "rule"},
            {"text": "user publishes on tuesdays", "kind": "memory"},
            {"text": "user lives in pune"}
        ]"#;
        let items = parse_items(raw).unwrap();
        assert_eq!(items.len(), 3);
        assert!(items[0].is_rule());
        assert!(!items[1].is_rule());
        assert!(!items[2].is_rule());
    }

    #[test]
    fn merge_learned_appends_and_dedups() {
        let (first, added) = merge_learned(
            "",
            &["Never use em-dashes.".into(), "Always write tests.".into()],
        );
        assert_eq!(added, 2);
        assert!(first.starts_with("# Learned\n"));
        assert!(first.contains("- Never use em-dashes.\n"));
        assert!(first.contains("- Always write tests.\n"));

        // Re-observation (case/whitespace-insensitive) is a no-op.
        let (second, added) = merge_learned(&first, &["never  use em-dashes.".into()]);
        assert_eq!(added, 0);
        assert_eq!(second, first);

        // A genuinely new line appends without disturbing hand edits.
        let edited = first.replace("Always write tests.", "Always write tests first.");
        let (third, added) = merge_learned(&edited, &["Prefer short names.".into()]);
        assert_eq!(added, 1);
        assert!(third.contains("Always write tests first."));
        assert!(third.ends_with("- Prefer short names.\n"));

        // Duplicates WITHIN one pass collapse to a single line.
        let (batch, added) = merge_learned(
            "",
            &["Prefer short names.".into(), "prefer  short names.".into()],
        );
        assert_eq!(added, 1);
        assert_eq!(batch.matches("- ").count(), 1);
    }

    #[test]
    fn content_text_reads_parts_and_plain_strings() {
        let parts = json!([
            { "type": "text", "text": "hello" },
            { "type": "image", "mime": "image/png", "data": "…" },
            { "type": "text", "text": "world" },
        ]);
        assert_eq!(content_text(Some(&parts)), "hello\nworld");
        let plain = json!("just a string");
        assert_eq!(content_text(Some(&plain)), "just a string");
    }
}
