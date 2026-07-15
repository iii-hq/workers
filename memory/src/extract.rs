//! Post-turn fact extraction: one `router::complete` call per completed
//! turn, off the hot path (spawned from the `harness::turn-completed`
//! handler, never inside a turn).
//!
//! ADD-only by design: extraction never rewrites or deletes existing facts.
//! The content fingerprint makes redelivery and re-observation idempotent —
//! a known fact is reinforced (`corroboration += 1`), never duplicated.
//! Memory's own function calls never reach this path: the transcript is
//! fetched with `roles: [user, assistant]`, and only text blocks are read.
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
use crate::types::{fingerprint, now_ms, Confidence, Fact, Provenance};

const EXTRACT_SYSTEM: &str = "You extract durable memory facts from a conversation excerpt.\n\
Return ONLY a JSON array (no prose, no code fences). Each element:\n\
{\"text\": string, \"entities\": [string], \"confidence\": \"extracted\"|\"inferred\"}\n\
Rules:\n\
- Only durable facts worth remembering across sessions: stable preferences, corrections, \
identities, project constants, standing instructions.\n\
- Never ephemeral state (current task progress, one-off values), never secrets, API keys, \
tokens, or passwords.\n\
- text: one self-contained sentence, max 200 characters, in the language of the source.\n\
- entities: short lowercase handles for the people/projects/tools the fact is about.\n\
- confidence: \"extracted\" when stated directly, \"inferred\" when derived.\n\
- Return [] when nothing qualifies. Most turns have nothing worth keeping.";

#[derive(Debug, Deserialize)]
struct ExtractedItem {
    text: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    confidence: Option<String>,
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
    // The pass succeeded (whatever it yielded): advance the cursor so the
    // next turn's extraction reads only newer messages.
    if let Some(id) = newest_id {
        cursor_set(&deps.iii, session_id, &id).await;
    }
    if items.is_empty() {
        return Ok(());
    }

    let store = deps.store().await;
    let (bank, _) = store
        .ensure_bank(&bank_name, None)
        .await
        .map_err(|e| e.to_string())?;

    let mut saved = 0usize;
    for item in items.into_iter().take(cfg.max_facts_per_turn) {
        let text = item.text.trim();
        if text.len() < 3 || text.len() > 500 {
            continue;
        }
        let id = fingerprint(text);
        let now = now_ms();
        let (fact, event) = match bank.get(&id).await {
            // Known fact re-observed: reinforce, never duplicate. A pinned
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
                Fact {
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
        match bank.commit(fact.clone()).await {
            Ok(CommitKind::Created | CommitKind::Updated) => {
                deps.emitter.item(event, &bank_name, &fact).await;
                saved += 1;
            }
            Err(e) => tracing::warn!(error = %e, "fact commit failed"),
        }
    }
    if saved > 0 {
        tracing::info!(session_id, bank = %bank_name, saved, "extraction pass saved facts");
        // Embed the new facts off this path; recall degrades gracefully
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
/// Page size while walking a session; bounded so a pathological session
/// costs at most PAGE_LIMIT * MAX_PAGES wire reads (never LLM tokens).
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 10;

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
    reply
        .pointer("/value/last_entry_id")
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
        match reply.get("next_cursor").and_then(Value::as_str) {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }

    let newest_id = collected.last().map(|(id, _, _)| id.clone());

    // Keep only entries strictly after the stored cursor; an unknown
    // cursor id (branch switch, trimmed history) falls back to the tail.
    let start = since
        .and_then(|s| collected.iter().position(|(id, _, _)| id == s))
        .map(|p| p + 1)
        .unwrap_or_else(|| collected.len().saturating_sub(window));

    let mut lines = Vec::new();
    for (_, role, text) in collected.into_iter().skip(start).rev().take(window) {
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() || trimmed.starts_with("<memory ") {
            continue;
        }
        let mut excerpt = trimmed;
        excerpt.truncate(2_000);
        lines.push(format!("{role}: {excerpt}"));
    }
    lines.reverse();
    Ok((lines.join("\n"), newest_id))
}

/// Concatenated text blocks of one message's content array.
pub fn content_text(content: Option<&Value>) -> String {
    let Some(blocks) = content.and_then(Value::as_array) else {
        // Tolerate plain-string content from non-harness agents.
        return content.and_then(Value::as_str).unwrap_or("").to_string();
    };
    blocks
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
    fn content_text_reads_blocks_and_plain_strings() {
        let blocks = json!([
            { "type": "text", "text": "hello" },
            { "type": "image", "mime": "image/png", "data": "…" },
            { "type": "text", "text": "world" },
        ]);
        assert_eq!(content_text(Some(&blocks)), "hello\nworld");
        let plain = json!("just a string");
        assert_eq!(content_text(Some(&plain)), "just a string");
    }
}
