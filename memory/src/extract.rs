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

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::events::ItemEvent;
use crate::hooks::ExtractJobInput;
use crate::store::CommitKind;
use crate::types::{fingerprint, local_date, now_ms, Confidence, Memory, Provenance};

const EXTRACT_SYSTEM: &str = "You extract durable memories from a conversation excerpt.\n\
Return ONLY a JSON array (no prose, no code fences). Each element:\n\
{\"text\": string, \"entities\": [string], \"tags\": [string], \
\"confidence\": \"extracted\"|\"inferred\", \"kind\": \"memory\"|\"rule\"}\n\
Rules:\n\
- Only durable content worth remembering across sessions: stable preferences, corrections, \
identities, project constants, standing instructions.\n\
- Never ephemeral state (current task progress, one-off values), never secrets, API keys, \
tokens, or passwords.\n\
- Each excerpt line starts with its message date in brackets. Dated events (appointments, \
deadlines, trips) are worth keeping even though they happen once: rewrite relative \
expressions (today, tomorrow, next Friday, in two weeks) as the absolute date, resolved \
against the date of the line that contains them, and write the event's own date in the \
text. Never add \"as of <date>\" or \"currently\" stamps to stable facts: each memory \
already carries its own date.\n\
- text: one self-contained sentence, max 200 characters, in the language of the source.\n\
- entities: short lowercase handles for the people/projects/tools the memory is about.\n\
- tags: 0-3 short lowercase topical labels for filtering within the bank (e.g. \"billing\", \
\"deploy\"); omit when nothing fits.\n\
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
    tags: Vec<String>,
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

/// Run one extraction pass for a completed turn. Errors are logged, never
/// propagated — extraction is a background enrichment, not a turn
/// dependency.
pub async fn run(deps: Arc<Deps>, job: ExtractJobInput) {
    if let Err(e) = try_run(&deps, &job).await {
        tracing::warn!(session_id = %job.session_id, error = %e, "extraction pass failed");
    }
}

/// Per-session pass locks; see [`session_pass`].
// ponytail: in-process — one worker replica. Replicated, it needs a FIFO
// queue grouped by session (`message_group_field: session_id`), which on
// the builtin adapter serializes every session instead.
static SESSION_PASSES: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The lock one session's passes take turns on. The queue runs jobs
/// concurrently, and two passes over one session would read the same
/// cursor: the older turn extracted twice (its corroboration inflated, or
/// into the newer turn's bank), and the slower pass writing the cursor
/// back behind the faster one's span.
fn session_pass(session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut passes = SESSION_PASSES.lock().unwrap_or_else(|e| e.into_inner());
    // Keep only sessions with a pass in flight or queued on the lock.
    passes.retain(|_, pass| Arc::strong_count(pass) > 1);
    passes.entry(session_id.to_string()).or_default().clone()
}

/// One pass over the job's span. Every error is returned, so the queue
/// retries the same span: a failed session lookup included — skipping
/// would lose the turn, guessing a bank would leak it.
pub async fn try_run(deps: &Deps, job: &ExtractJobInput) -> Result<(), String> {
    let session_id = job.session_id.as_str();
    let pass = session_pass(session_id);
    let _pass = pass.lock().await;
    let cfg = deps.config().await;
    let bank_name = match (&job.bank, job.skip) {
        (_, true) => None,
        (Some(bank), false) => Some(bank.clone()),
        (None, false) => crate::scope::resolve(deps, session_id, None).await?,
    };
    // Deleted after the turn ran: the pass must not bring it back.
    let bank_name = match bank_name {
        Some(b) if b != cfg.default_bank && deps.store().await.is_deleted(&b) => None,
        other => other,
    };

    let since = cursor_get(&deps.iii, session_id).await?;
    let (entries, until_seen) = fetch_span(
        &deps.iii,
        session_id,
        since.as_deref(),
        job.until.as_deref(),
        cfg.extraction_window,
    )
    .await?;
    let Some(bank_name) = bank_name else {
        // The turn ran without memory (an invalid or deleted bank):
        // step past it, so a later turn's pass never sweeps it into that
        // turn's bank — but never past a turn end it could not find.
        let newest = entries.last().map(|(id, ..)| id.as_str());
        if let Some(id) = newest.filter(|_| until_seen || job.until.is_none()) {
            cursor_set(&deps.iii, session_id, id).await;
        }
        return Ok(());
    };
    let (transcript, newest_id, content_len) = excerpt(entries, cfg.extraction_window);
    // Measured on message text, not the dated transcript: the per-line
    // `role [date]: ` prefix alone would clear any threshold.
    if content_len < 20 {
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
    let (bank, created) = store
        .ensure_bank(&bank_name, None)
        .await
        .map_err(|e| e.to_string())?;
    if created {
        deps.emitter.bank("created", &bank_name).await;
    }

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
    for mut item in items.into_iter().take(cfg.max_memories_per_turn) {
        // Redact before the fingerprint and before normalization, like
        // memory::save.
        item.text = crate::redact::redact(item.text.trim()).text;
        crate::redact::drop_secrets(&mut item.entities);
        crate::redact::drop_secrets(&mut item.tags);
        let text = item.text.as_str();
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
                f.stated_at = Some(now);
                f.revision += 1;
                (f, ItemEvent::Updated)
            }
            Some(_) => continue,
            None => (
                new_memory(item, session_id, newest_id.as_deref(), now),
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
        crate::embed_client::spawn_backfill(deps, &bank_name);
    }
    Ok(())
}

/// A fresh record for one extracted item. `entry_id` is the newest
/// transcript entry of the extracted span — the turn's last message — so
/// the memory points back at the turn it came from.
fn new_memory(item: ExtractedItem, session_id: &str, entry_id: Option<&str>, now: u64) -> Memory {
    let text = item.text.trim().to_string();
    Memory {
        id: fingerprint(&text),
        text,
        entities: item
            .entities
            .into_iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .take(8)
            .collect(),
        tags: item
            .tags
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
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
            entry_id: entry_id.map(str::to_string),
            agent: None,
        }),
        created_at: now,
        updated_at: now,
        invalid_at: None,
        superseded_by: None,
        revision: 0,
        stated_at: Some(now),
    }
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

/// Last processed entry id for a session, from the state worker. No cursor
/// yet, or no state worker at all, is `None` — the pass falls back to the
/// newest `window` messages. A transient failure is an error, so the job
/// retries instead of re-reading old turns into this turn's bank.
async fn cursor_get(iii: &IIIClient, session_id: &str) -> Result<Option<String>, String> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": CURSOR_SCOPE, "key": session_id }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await;
    let reply = match reply {
        Ok(reply) => reply,
        Err(e) if transient(&e) => return Err(format!("state::get: {e}")),
        Err(_) => return Ok(None),
    };
    // state::get returns the stored value at the top level; older engines
    // wrapped it under `value` — accept both.
    Ok(reply
        .pointer("/last_entry_id")
        .or_else(|| reply.pointer("/value/last_entry_id"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// A failure worth retrying. `function_not_found` is not: it is also what
/// a stack without a state worker answers, every time.
fn transient(e: &Error) -> bool {
    match e {
        Error::Timeout | Error::NotConnected | Error::WebSocket(_) => true,
        Error::Remote { code, .. } => code == "invocation_error",
        _ => false,
    }
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

/// The [`Span`] of user/assistant messages after `since` through `until`,
/// and whether `until` was on the path. Walks the session page by page —
/// `session::messages` returns root→leaf, so the new turn lives on the
/// LAST page. `roles` narrowing also excludes custom + function_result
/// entries, so memory's own calls never feed back into extraction.
async fn fetch_span(
    iii: &IIIClient,
    session_id: &str,
    since: Option<&str>,
    until: Option<&str>,
    window: usize,
) -> Result<(Vec<Entry>, bool), String> {
    let mut span = Span::new(since, until, window);
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
            span.push(items.iter().filter_map(parse_entry));
        }
        if span.done() {
            break;
        }
        match reply.get("next_cursor").and_then(Value::as_str) {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    Ok(span.finish())
}

/// The dated excerpt of a span (its newest `window` entries), the span's
/// newest entry id, and the kept message text length (excluding the
/// per-line date prefix).
fn excerpt(entries: Vec<Entry>, window: usize) -> (String, Option<String>, usize) {
    let newest_id = entries.last().map(|(id, ..)| id.clone());
    let start = entries.len().saturating_sub(window);
    let mut content_len = 0;
    let lines: Vec<String> = entries
        .into_iter()
        .skip(start)
        .filter_map(|(_, role, text, ts)| {
            let line = transcript_line(&role, &text, ts)?;
            content_len += text.trim().len();
            Some(line)
        })
        .collect();
    (lines.join("\n"), newest_id, content_len)
}

/// One transcript entry: `(entry_id, role, text, timestamp_ms)`.
type Entry = (String, String, String, u64);

/// What one pass reads off a root → leaf walk: the entries after the cursor
/// `since`, through `until` (the end of the job's turn), or the newest
/// `window` of them when the cursor is never seen (no cursor yet, a branch
/// switch). Stopping at `until` keeps a late job from sweeping the NEXT
/// turn into its own, maybe different, bank.
struct Span<'a> {
    since: Option<&'a str>,
    until: Option<&'a str>,
    window: usize,
    entries: Vec<Entry>,
    since_seen: bool,
    until_seen: bool,
    /// The cursor sits past `until`: an earlier pass already read this turn.
    stale: bool,
}

impl<'a> Span<'a> {
    fn new(since: Option<&'a str>, until: Option<&'a str>, window: usize) -> Self {
        Self {
            since,
            until,
            window,
            entries: Vec::new(),
            since_seen: false,
            until_seen: false,
            stale: false,
        }
    }

    fn push(&mut self, page: impl IntoIterator<Item = Entry>) {
        for entry in page {
            if self.until_seen {
                // Only watched for the cursor: one after `until` means stale.
                self.stale |= self.since == Some(entry.0.as_str());
                continue;
            }
            self.until_seen = self.until == Some(entry.0.as_str());
            self.entries.push(entry);
        }
        // Bound RAM on long sessions: everything at or before the stored
        // cursor is already extracted, so once the cursor id streams past
        // only what follows it is kept; without a cursor only the window
        // tail is ever read.
        if let Some(since) = self.since {
            if let Some(pos) = self.entries.iter().position(|(id, ..)| id == since) {
                self.since_seen = true;
                self.entries.drain(..=pos);
                return;
            }
            if self.since_seen {
                return;
            }
        }
        let excess = self.entries.len().saturating_sub(self.window);
        self.entries.drain(..excess);
    }

    /// Later pages can change nothing: the turn's end is in, and the cursor
    /// was found before it (or there is none to look for).
    fn done(&self) -> bool {
        self.until_seen && (self.since.is_none() || self.since_seen)
    }

    /// The entries to read, and whether the turn's end was on the path.
    fn finish(self) -> (Vec<Entry>, bool) {
        if self.stale {
            return (Vec::new(), true);
        }
        (self.entries, self.until_seen)
    }
}

/// The end of `turn_id` on a `session::messages-tail` page: the newest
/// user/assistant entry the turn wrote (`origin.turn_id`). Not merely the
/// newest entry — the harness drains a queued user message, and may open
/// the next turn, before it emits turn-completed. Function results and
/// custom entries never reach the pass (`roles` narrows them out), so they
/// cannot end it.
fn newest_turn_entry(reply: &Value, turn_id: &str) -> Option<String> {
    reply
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|item| {
            item.pointer("/origin/turn_id").and_then(Value::as_str) == Some(turn_id)
                && matches!(
                    item.pointer("/message/role").and_then(Value::as_str),
                    Some("user" | "assistant")
                )
        })?
        .get("entry_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Where `turn_id` ended, read when it completes; `None` when
/// session-manager cannot say or the turn wrote nothing in view (the job
/// then reads to the newest entry, as before turn ends were tracked).
pub async fn turn_end(iii: &IIIClient, session_id: &str, turn_id: &str) -> Option<String> {
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "session::messages-tail".into(),
            payload: json!({
                "session_id": session_id,
                "limit": 10,
                "include_custom": false,
                "include_image_data": false,
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok()?;
    newest_turn_entry(&reply, turn_id)
}

/// One `session::messages` item as `(entry_id, role, text, timestamp_ms)`;
/// `None` for entries without a message (custom entries). A missing or
/// zero timestamp falls back to the pass's own clock.
fn parse_entry(item: &Value) -> Option<(String, String, String, u64)> {
    let message = item.get("message")?;
    let entry_id = item
        .get("entry_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let text = content_text(message.get("content"));
    let ts = message
        .get("timestamp")
        .and_then(Value::as_u64)
        .filter(|&t| t > 0)
        .unwrap_or_else(now_ms);
    Some((entry_id, role, text, ts))
}

/// One excerpt line, `role [YYYY-MM-DD Weekday]: text`, dated by its OWN
/// message so relative dates resolve right even when the excerpt spans days
/// (an imported conversation, a first pass over an old session). Empty
/// turns and our own injected `<memory …>` blocks are dropped.
fn transcript_line(role: &str, text: &str, ts: u64) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with("<memory ") {
        return None;
    }
    // Redacted before it reaches the extraction model: a secret it never
    // sees is one it cannot copy into a memory.
    // Redact before the cut: a key crossing the 2000-char clamp would
    // otherwise reach the model half-visible.
    let redacted = crate::redact::redact(trimmed).text;
    let excerpt = &redacted[..clamp_char_boundary(&redacted, 2_000)];
    Some(format!(
        "{role} [{}]: {excerpt}",
        local_date(ts, "%Y-%m-%d %A")
    ))
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

    fn entry(id: &str) -> Entry {
        (id.into(), "user".into(), format!("text of {id}"), 1)
    }

    /// Walk `ids` as two pages, like `session::messages` root → leaf.
    fn walk(
        ids: &[&str],
        since: Option<&str>,
        until: Option<&str>,
        window: usize,
    ) -> (Vec<String>, bool) {
        let mut span = Span::new(since, until, window);
        let (a, b) = ids.split_at(ids.len() / 2);
        span.push(a.iter().map(|id| entry(id)));
        if !span.done() {
            span.push(b.iter().map(|id| entry(id)));
        }
        let (entries, until_seen) = span.finish();
        (entries.into_iter().map(|e| e.0).collect(), until_seen)
    }

    #[test]
    fn a_span_runs_from_the_cursor_through_the_turns_end() {
        let ids = ["e1", "e2", "e3", "e4", "e5", "e6"];
        assert_eq!(
            walk(&ids, Some("e2"), Some("e4"), 12),
            (vec!["e3".into(), "e4".into()], true),
            "the next turn (e5, e6) is not this job's"
        );
        assert_eq!(
            walk(&ids, Some("e2"), None, 12),
            (
                vec!["e3".into(), "e4".into(), "e5".into(), "e6".into()],
                false
            ),
            "a job without a turn end reads to the newest entry"
        );
    }

    #[test]
    fn a_cursor_at_or_past_the_turns_end_leaves_nothing_to_read() {
        let ids = ["e1", "e2", "e3", "e4", "e5", "e6"];
        assert_eq!(walk(&ids, Some("e5"), Some("e3"), 12), (vec![], true));
        assert_eq!(walk(&ids, Some("e3"), Some("e3"), 12), (vec![], true));
    }

    #[test]
    fn without_a_known_cursor_the_window_ends_at_the_turns_end() {
        let ids = ["e1", "e2", "e3", "e4", "e5", "e6"];
        assert_eq!(
            walk(&ids, None, Some("e4"), 2),
            (vec!["e3".into(), "e4".into()], true)
        );
        assert_eq!(
            walk(&ids, Some("gone"), Some("e4"), 2),
            (vec!["e3".into(), "e4".into()], true),
            "a cursor off the active path falls back to the window"
        );
        assert_eq!(
            walk(&ids, Some("e2"), Some("absent"), 12),
            (
                vec!["e3".into(), "e4".into(), "e5".into(), "e6".into()],
                false
            ),
            "a turn end off the path reads to the newest entry"
        );
    }

    fn remote(code: &str) -> Error {
        Error::Remote {
            code: code.into(),
            message: String::new(),
            stacktrace: None,
        }
    }

    #[test]
    fn only_a_transient_failure_retries_the_cursor_read() {
        assert!(transient(&Error::Timeout));
        assert!(transient(&Error::NotConnected));
        assert!(transient(&remote("invocation_error")));
        // No state worker at all: no cursor, as before.
        assert!(!transient(&remote("function_not_found")));
    }

    #[tokio::test]
    async fn passes_over_one_session_take_turns() {
        let first = session_pass("s");
        let held = first.lock().await;
        assert!(
            Arc::ptr_eq(&first, &session_pass("s")),
            "same session, same pass"
        );
        assert!(!Arc::ptr_eq(&first, &session_pass("other")));
        drop(held);
        drop(first);
        // Idle passes are dropped, so the map tracks only sessions in flight.
        let _ = session_pass("third");
        let passes = SESSION_PASSES.lock().unwrap();
        assert!(!passes.contains_key("other"));
    }

    #[test]
    fn a_turns_end_is_its_own_newest_user_or_assistant_entry() {
        // The harness drains a queued message (and may open the next turn)
        // before it emits turn-completed: neither ends THIS turn.
        let reply = json!({ "messages": [
            { "entry_id": "u1", "message": { "role": "user" } },
            { "entry_id": "a1", "message": { "role": "assistant" }, "origin": { "turn_id": "t1" } },
            { "entry_id": "r1", "message": { "role": "function_result" }, "origin": { "turn_id": "t1" } },
            { "entry_id": "c1", "custom": {}, "origin": { "turn_id": "t1" } },
            { "entry_id": "q2", "message": { "role": "user" } },
            { "entry_id": "a2", "message": { "role": "assistant" }, "origin": { "turn_id": "t2" } },
        ]});
        assert_eq!(newest_turn_entry(&reply, "t1").as_deref(), Some("a1"));
        assert_eq!(
            newest_turn_entry(&reply, "t9"),
            None,
            "nothing of the turn in view"
        );
        assert_eq!(newest_turn_entry(&json!({ "messages": [] }), "t1"), None);
    }

    #[test]
    fn parse_entry_reads_role_text_and_message_timestamp() {
        let item = json!({
            "entry_id": "e_1",
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "hi" }],
                "timestamp": 1_760_443_200_000_i64,
            },
        });
        let (id, role, text, ts) = parse_entry(&item).unwrap();
        assert_eq!(
            (id.as_str(), role.as_str(), text.as_str(), ts),
            ("e_1", "user", "hi", 1_760_443_200_000)
        );

        // No timestamp: the pass's own clock stands in.
        let before = now_ms();
        let bare =
            json!({ "entry_id": "e_2", "message": { "role": "assistant", "content": "ok" } });
        assert!(parse_entry(&bare).unwrap().3 >= before);
        // Custom entries carry no message.
        assert!(parse_entry(&json!({ "entry_id": "e_3", "custom": {} })).is_none());
    }

    #[test]
    fn transcript_lines_carry_their_own_local_date() {
        // Noon UTC: the same calendar day from UTC-11 to UTC+11.
        let day1 = 1_760_443_200_000;
        let day2 = day1 + 2 * 86_400_000;
        let a = transcript_line("user", "o deploy é amanhã", day1).unwrap();
        let b = transcript_line("assistant", "ok", day2).unwrap();
        let d1 = local_date(day1, "%Y-%m-%d %A");
        let d2 = local_date(day2, "%Y-%m-%d %A");
        assert_ne!(d1, d2);
        assert_eq!(a, format!("user [{d1}]: o deploy é amanhã"));
        assert_eq!(b, format!("assistant [{d2}]: ok"));
        // Injected memory blocks and empty turns never reach the excerpt.
        assert!(transcript_line("user", "<memory bank=\"m\">…</memory>", day1).is_none());
        // A secret pasted in chat never reaches the extraction model.
        // Assembled at runtime: no literal key shape in the source.
        let key = format!("sk-ant-api03-{}", "a1B2c3D4e5".repeat(3));
        let pasted = transcript_line("user", &format!("my key is {key}"), day1).unwrap();
        assert!(
            pasted.ends_with("my key is [REDACTED:anthropic_key]"),
            "{pasted}"
        );
        // A key crossing the 2000-char cut is redacted, not half-sent.
        let long = format!("{} {key}", "x".repeat(1_990));
        let cut = transcript_line("user", &long, day1).unwrap();
        assert!(!cut.contains("sk-ant-api03"), "{cut}");
        assert!(transcript_line("user", "   ", day1).is_none());
    }

    #[test]
    fn new_memory_records_the_span_entry_and_normalizes_fields() {
        let item = ExtractedItem {
            text: "  user prefers tea  ".into(),
            entities: vec![" Mike ".into(), "".into()],
            tags: vec!["Drinks".into()],
            confidence: Some("inferred".into()),
            kind: None,
        };
        let m = new_memory(item, "s_1", Some("e_9"), 42);
        assert_eq!(m.text, "user prefers tea");
        assert_eq!(m.id, fingerprint("user prefers tea"));
        assert_eq!(m.entities, vec!["mike"]);
        assert_eq!(m.tags, vec!["drinks"]);
        assert_eq!(m.confidence, Confidence::Inferred);
        let src = m.source.unwrap();
        assert_eq!(src.session_id.as_deref(), Some("s_1"));
        assert_eq!(src.entry_id.as_deref(), Some("e_9"));
        assert_eq!((m.created_at, m.updated_at), (42, 42));
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
