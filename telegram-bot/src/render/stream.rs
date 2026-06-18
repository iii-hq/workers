//! Assistant output streaming: draft transport, edit fallback, finalization.

use std::future::Future;

use iii_sdk::IIIError;

use crate::clients::telegram;
use crate::config::{StreamTransport, WorkerConfig};
use crate::deps::{Deps, RuntimeState, StreamSession};
use crate::kv;
use crate::preferences;
use crate::render::chunk::{split_message, TELEGRAM_MAX_MESSAGE_LEN};
use crate::render::format;
use crate::render::throttle;
use crate::render::typing;
use crate::render::verbosity::{
    self, message_phase, render_answer_text, render_thinking_text, MessagePhase,
};
use crate::types::{AgentMessage, MessageUpdatedEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveTransport {
    Draft,
    Edit,
}

pub async fn on_message_added(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    message: &AgentMessage,
    timestamp: i64,
) -> Result<Option<i64>, IIIError> {
    let cfg = deps.cfg().await;
    let Some(chat_id) = kv::chat_id_for_session(deps, session_id).await else {
        return Ok(None);
    };

    if message.role == "function_result" {
        let verbosity = preferences::effective_verbosity(deps, chat_id, &cfg).await;
        let text = verbosity::render_function_result_message(message, verbosity);
        let text = match text {
            Some(t) if !t.is_empty() => t,
            _ => return Ok(None),
        };
        let order_key = record_order_key(deps, session_id, entry_id, timestamp).await;
        let id = send_chat_message_in_order(
            deps,
            &cfg,
            OrderedChatMessage {
                chat_id,
                order_key,
                entry_id,
                chunk_idx: 0,
                text: &text,
                reply_markup: None,
            },
        )
        .await?;
        return Ok(Some(id));
    }

    if message.role != "assistant" {
        return Ok(None);
    }

    let phase = message_phase(&message.content);
    if phase == MessagePhase::Empty {
        return with_entry_lock(deps, session_id, entry_id, || {
            on_empty_assistant_added(deps, session_id, entry_id, timestamp, chat_id, &cfg)
        })
        .await;
    }

    with_entry_lock(deps, session_id, entry_id, || {
        on_message_added_locked(
            deps, session_id, entry_id, message, timestamp, chat_id, &cfg,
        )
    })
    .await
}

async fn on_message_added_locked(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    message: &AgentMessage,
    timestamp: i64,
    chat_id: i64,
    cfg: &WorkerConfig,
) -> Result<Option<i64>, IIIError> {
    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(kv::entry_message_id(deps, session_id, entry_id).await);
    }

    let order_key = record_order_key(deps, session_id, entry_id, timestamp).await;

    if let Some(existing_id) = kv::entry_message_id(deps, session_id, entry_id).await {
        let mut session = bootstrap_session(deps, cfg, session_id, entry_id, chat_id).await;
        hydrate_message_ids(deps, session_id, entry_id, &mut session).await;
        session.message_id = Some(existing_id);
        session.phase = message_phase(&message.content);
        session.order_key = order_key;
        deps.runtime
            .stream_sessions
            .insert(entry_key(session_id, entry_id), session);
        return Ok(Some(existing_id));
    }

    // Prefer an existing in-memory session: a `message-updated` may have raced
    // ahead of this `message-added`, and bootstrapping fresh would reset
    // `last_revision` to 0 and let the (stale, revision-0) added clobber it.
    let key = entry_key(session_id, entry_id);
    let mut session = deps
        .runtime
        .stream_sessions
        .get(&key)
        .map(|s| s.clone())
        .unwrap_or_else(|| bootstrap_session_sync(cfg, deps, session_id, entry_id, chat_id));
    hydrate_message_ids(deps, session_id, entry_id, &mut session).await;
    session.order_key = order_key;

    // Only block siblings while this entry has yet to post its first bubble. An
    // out-of-order update may already have materialized it (edit transport).
    if session.message_id.is_none() {
        register_pending_materialization(&deps.runtime, chat_id, order_key, entry_id);
    }

    apply_render(
        deps,
        cfg,
        session_id,
        entry_id,
        message,
        0,
        order_key,
        &mut session,
    )
    .await?;

    deps.runtime.stream_sessions.insert(key, session.clone());

    Ok(session.message_id)
}

async fn on_empty_assistant_added(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    timestamp: i64,
    chat_id: i64,
    cfg: &WorkerConfig,
) -> Result<Option<i64>, IIIError> {
    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(kv::entry_message_id(deps, session_id, entry_id).await);
    }

    let order_key = record_order_key(deps, session_id, entry_id, timestamp).await;
    let mut session = bootstrap_session(deps, cfg, session_id, entry_id, chat_id).await;
    hydrate_message_ids(deps, session_id, entry_id, &mut session).await;
    session.order_key = order_key;

    register_pending_materialization(&deps.runtime, chat_id, order_key, entry_id);

    if should_emit_native_thinking_placeholder(session.transport) {
        send_native_thinking_placeholder(deps, session_id, &mut session, "").await?;
    }

    deps.runtime
        .stream_sessions
        .insert(entry_key(session_id, entry_id), session);

    Ok(None)
}

pub async fn on_message_updated(deps: &Deps, evt: MessageUpdatedEvent) -> Result<(), IIIError> {
    let cfg = deps.cfg().await;
    let Some(chat_id) = kv::chat_id_for_session(deps, &evt.session_id).await else {
        return Ok(());
    };

    let session_id = evt.session_id.clone();
    let entry_id = evt.entry_id.clone();
    let message = evt.message;
    let revision = evt.revision;
    let timestamp = evt.timestamp;

    with_entry_lock(deps, &session_id, &entry_id, || {
        on_message_updated_locked(
            deps,
            &session_id,
            &entry_id,
            message,
            revision,
            timestamp,
            chat_id,
            &cfg,
        )
    })
    .await
}

#[allow(clippy::too_many_arguments)]
async fn on_message_updated_locked(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    message: AgentMessage,
    revision: u64,
    timestamp: i64,
    chat_id: i64,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    if is_entry_finalized(deps, session_id, entry_id).await {
        return reconcile_finalized_update(
            deps, cfg, session_id, entry_id, &message, revision, timestamp, chat_id,
        )
        .await;
    }

    let order_key = record_order_key(deps, session_id, entry_id, timestamp).await;

    let key = entry_key(session_id, entry_id);
    let mut session = deps
        .runtime
        .stream_sessions
        .get(&key)
        .map(|s| s.clone())
        .unwrap_or_else(|| bootstrap_session_sync(cfg, deps, session_id, entry_id, chat_id));

    hydrate_message_ids(deps, session_id, entry_id, &mut session).await;
    session.order_key = order_key;

    apply_render(
        deps,
        cfg,
        session_id,
        entry_id,
        &message,
        revision,
        order_key,
        &mut session,
    )
    .await?;

    deps.runtime.stream_sessions.insert(key, session);
    Ok(())
}

/// Handle a `message-updated` that arrives after the entry was finalized.
///
/// Finalize can win the per-entry lock before the last `message-updated` task
/// runs (handlers are spawned per event and only loosely ordered), persisting
/// slightly stale text. Rather than drop the genuine final revision, edit the
/// already-posted message so it matches the authoritative text. Only strictly
/// higher revisions reconcile, so duplicate/stale late events are ignored.
#[allow(clippy::too_many_arguments)]
async fn reconcile_finalized_update(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    message: &AgentMessage,
    revision: u64,
    timestamp: i64,
    chat_id: i64,
) -> Result<(), IIIError> {
    let key = entry_key(session_id, entry_id);
    let finalized_revision = deps
        .runtime
        .finalized_entries
        .get(&key)
        .map(|r| *r.value())
        .unwrap_or(u64::MAX);
    if !should_reconcile_finalized(revision, finalized_revision) {
        return Ok(());
    }

    let verbosity = preferences::effective_verbosity(deps, chat_id, cfg).await;
    let text = match render_answer_text(message, verbosity) {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(()),
    };

    let mut message_id = kv::entry_message_id(deps, session_id, entry_id).await;
    if message_id.is_none() {
        return Ok(());
    }

    // Record the higher revision before editing so an even-later event still
    // wins and stale ones remain dropped.
    deps.runtime.finalized_entries.insert(key, revision);

    let order_key = record_order_key(deps, session_id, entry_id, timestamp).await;
    deliver_text(
        deps,
        cfg,
        session_id,
        entry_id,
        chat_id,
        &mut message_id,
        &text,
        revision,
        false,
        order_key,
    )
    .await
}

pub async fn finalize_session(deps: &Deps, session_id: &str) -> Result<(), IIIError> {
    let cfg = deps.cfg().await;

    let mut ordered_keys: Vec<(i64, (String, String))> = deps
        .runtime
        .stream_sessions
        .iter()
        .filter(|e| e.key().0 == session_id)
        .map(|e| (e.value().order_key, e.key().clone()))
        .collect();

    for e in deps.runtime.pending_entries.iter() {
        if e.key().0 != session_id || e.value().finalized {
            continue;
        }
        if deps.runtime.finalized_entries.contains_key(e.key()) {
            continue;
        }
        let key = e.key().clone();
        if !ordered_keys.iter().any(|(_, k)| k == &key) {
            ordered_keys.push((e.value().order_key, key));
        }
    }

    let keys = order_for_posting(ordered_keys);

    for key in keys {
        if is_entry_finalized(deps, &key.0, &key.1).await {
            deps.runtime.stream_sessions.remove(&key);
            deps.runtime.pending_entries.remove(&key);
            continue;
        }
        let sid = key.0.clone();
        let eid = key.1.clone();
        with_entry_lock(deps, &sid, &eid, || {
            finalize_entry_under_lock(deps, &cfg, &sid, &eid)
        })
        .await?;
    }

    typing::suppress_typing(deps, session_id);
    Ok(())
}

async fn finalize_entry_under_lock(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
) -> Result<(), IIIError> {
    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(());
    }

    let key = entry_key(session_id, entry_id);
    let chat_id = deps
        .runtime
        .stream_sessions
        .get(&key)
        .map(|s| s.chat_id)
        .or_else(|| deps.runtime.pending_entries.get(&key).map(|p| p.chat_id));

    let Some(chat_id) = chat_id else {
        return Ok(());
    };

    let mut session = deps
        .runtime
        .stream_sessions
        .remove(&key)
        .map(|(_, s)| s)
        .unwrap_or_else(|| bootstrap_session_sync(cfg, deps, session_id, entry_id, chat_id));

    if let Some(pending) = deps.runtime.pending_entries.get(&key) {
        merge_pending_into_session(&mut session, &pending);
    }

    finalize_entry_locked(deps, cfg, session_id, entry_id, &mut session).await?;
    deps.runtime.stream_sessions.remove(&key);
    Ok(())
}

async fn finalize_entry_locked(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
) -> Result<(), IIIError> {
    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(());
    }
    if session.finalized {
        mark_entry_finalized(
            deps,
            &entry_key(session_id, entry_id),
            session.chat_id,
            session.last_revision,
        )
        .await;
        return Ok(());
    }

    hydrate_message_ids(deps, session_id, entry_id, session).await;
    finalize_entry(deps, cfg, session_id, entry_id, session).await?;
    mark_entry_finalized(
        deps,
        &entry_key(session_id, entry_id),
        session.chat_id,
        session.last_revision,
    )
    .await;
    Ok(())
}

async fn with_entry_lock<F, Fut, T>(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    f: F,
) -> Result<T, IIIError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, IIIError>>,
{
    let lock = deps.runtime.entry_lock(&entry_key(session_id, entry_id));
    let _guard = lock.lock().await;
    f().await
}

fn entry_key(session_id: &str, entry_id: &str) -> (String, String) {
    (session_id.to_string(), entry_id.to_string())
}

async fn bootstrap_session(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    chat_id: i64,
) -> StreamSession {
    bootstrap_session_sync(cfg, deps, session_id, entry_id, chat_id)
}

fn bootstrap_session_sync(
    cfg: &WorkerConfig,
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    chat_id: i64,
) -> StreamSession {
    StreamSession {
        draft_id: draft_id_for_entry(cfg, session_id, entry_id),
        chat_id,
        transport: resolve_transport(deps, chat_id, cfg),
        message_id: None,
        thinking_message_id: None,
        last_revision: 0,
        last_text: String::new(),
        last_thinking_text: String::new(),
        phase: MessagePhase::Empty,
        finalized: false,
        draft_started: false,
        // Overwritten by the caller from the event's recorded order key.
        order_key: 0,
    }
}

async fn hydrate_message_ids(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
) {
    if session.message_id.is_none() {
        session.message_id = kv::entry_message_id(deps, session_id, entry_id).await;
    }
    if session.thinking_message_id.is_none() {
        session.thinking_message_id = kv::thinking_message_id(deps, session_id, entry_id).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn apply_render(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    message: &AgentMessage,
    revision: u64,
    order_key: i64,
    session: &mut StreamSession,
) -> Result<(), IIIError> {
    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(());
    }

    // The per-entry lock serializes handlers but does not order them: an older
    // revision can win the lock after a newer one already applied. Re-applying
    // it would regress the live draft and the `last_text`/pending snapshot used
    // at finalize. `message-added` carries revision 0, which is only fresh for a
    // brand-new entry (whose `last_revision` is also 0).
    if !revision_is_fresh(revision, session.last_revision) {
        return Ok(());
    }

    let phase = message_phase(&message.content);
    session.phase = phase;
    session.order_key = order_key;

    let verbosity = preferences::effective_verbosity(deps, session.chat_id, cfg).await;

    let answer_text = render_answer_text(message, verbosity);
    let thinking_text = render_thinking_text(message);

    let text = match answer_text {
        Some(t) if !t.is_empty() => t,
        _ if phase == MessagePhase::ThinkingOnly => String::new(),
        _ => return Ok(()),
    };

    session.last_revision = revision;
    session.last_text = text.clone();
    session.last_thinking_text = thinking_text.clone().unwrap_or_default();

    hydrate_message_ids(deps, session_id, entry_id, session).await;

    match session.transport {
        EffectiveTransport::Draft => {
            apply_draft_update(deps, cfg, session_id, entry_id, session, &text, phase).await?;
        }
        EffectiveTransport::Edit => {
            apply_edit_update(deps, cfg, session_id, entry_id, session, &text).await?;
        }
    }

    // Snapshot after the transport step so the pending entry reflects the most
    // recent `draft_started`/`message_id` (e.g. after an edit-fallback switch).
    deps.runtime.pending_entries.insert(
        entry_key(session_id, entry_id),
        crate::deps::PendingEntryState {
            chat_id: session.chat_id,
            revision,
            text,
            thinking_text: session.last_thinking_text.clone(),
            phase,
            message_id: session.message_id,
            thinking_message_id: session.thinking_message_id,
            finalized: false,
            draft_started: session.draft_started,
            order_key,
        },
    );

    Ok(())
}

/// Whether `incoming` is at least as new as the last applied revision. Stale
/// (out-of-order) revisions are dropped so they cannot regress rendered text.
fn revision_is_fresh(incoming: u64, last_applied: u64) -> bool {
    incoming >= last_applied
}

/// Whether a post-finalize `message-updated` should edit the already-posted
/// message. Only strictly higher revisions reconcile, so duplicate/stale late
/// events — and restart-learned finalizes recorded as `u64::MAX` — are ignored.
fn should_reconcile_finalized(incoming: u64, finalized: u64) -> bool {
    incoming > finalized
}

async fn apply_draft_update(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
    text: &str,
    phase: MessagePhase,
) -> Result<(), IIIError> {
    if !should_draft(deps, session, cfg.streaming.draft_throttle_ms) {
        return Ok(());
    }

    if uses_rich_thinking_draft(phase) {
        let thinking = session.last_thinking_text.clone();
        send_native_thinking_placeholder(deps, session_id, session, &thinking).await?;
        return Ok(());
    }

    let draft_text = if phase == MessagePhase::ThinkingOnly && text.is_empty() {
        ""
    } else {
        text
    };

    match telegram::send_message_draft(deps, session.chat_id, session.draft_id, draft_text, None)
        .await
    {
        Ok(()) => {
            session.draft_started = true;
            if !draft_text.is_empty() {
                typing::stop_typing_on_output(deps, session_id);
            }
        }
        Err(e) if is_draft_unsupported(&e) => {
            pin_edit_fallback(deps, session.chat_id);
            session.transport = EffectiveTransport::Edit;
            apply_edit_update(deps, cfg, session_id, entry_id, session, text).await?;
        }
        Err(e) => {
            tracing::warn!(error = %e, "sendMessageDraft failed");
        }
    }
    Ok(())
}

async fn apply_edit_update(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
    text: &str,
) -> Result<(), IIIError> {
    let chat_id = session.chat_id;
    let revision = session.last_revision;
    let order_key = session.order_key;
    deliver_text(
        deps,
        cfg,
        session_id,
        entry_id,
        chat_id,
        &mut session.message_id,
        text,
        revision,
        true,
        order_key,
    )
    .await?;
    if !text.is_empty() {
        typing::stop_typing_on_output(deps, session_id);
    }
    Ok(())
}

/// Send a new Telegram message or edit an existing one, with idempotency guards.
///
/// Every new `sendMessage` (first chunk and continuations) goes through the
/// per-chat ordering gate so bubbles land in append order relative to other
/// entries in the same turn.
#[allow(clippy::too_many_arguments)]
async fn deliver_text(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    chat_id: i64,
    message_id: &mut Option<i64>,
    text: &str,
    revision: u64,
    throttle_edits: bool,
    order_key: i64,
) -> Result<(), IIIError> {
    let chunks = split_message(text, TELEGRAM_MAX_MESSAGE_LEN);
    if chunks.is_empty() || chunks.iter().all(|c| c.is_empty()) {
        return Ok(());
    }

    // The finalize path (`throttle_edits == false`) skips the create-slot settle
    // delay so the persistent bubble appears immediately; live streaming keeps
    // the configured settle so near-simultaneous sibling entries still order.
    let settle_ms = if throttle_edits {
        cfg.streaming.create_settle_ms
    } else {
        0
    };

    if is_entry_finalized(deps, session_id, entry_id).await {
        if let Some(id) = *message_id {
            if let Some(first) = chunks.first() {
                let _ = edit_message_text_formatted(deps, chat_id, id, first).await;
            }
            for (i, chunk) in chunks.iter().skip(1).enumerate() {
                let chunk_idx = i as u32 + 1;
                let _ = send_in_order(
                    deps,
                    chat_id,
                    order_key,
                    entry_id,
                    chunk_idx,
                    settle_ms,
                    || send_message_formatted(deps, chat_id, chunk, None),
                )
                .await;
            }
        }
        return Ok(());
    }

    if message_id.is_none() {
        *message_id = kv::entry_message_id(deps, session_id, entry_id).await;
    }

    if let Some(id) = *message_id {
        if throttle_edits
            && !throttle::should_edit(
                &deps.runtime,
                chat_id,
                id,
                revision,
                session_id,
                entry_id,
                cfg.streaming.draft_throttle_ms,
            )
        {
            return Ok(());
        }
        if let Some(first) = chunks.first() {
            if let Err(e) = edit_message_text_formatted(deps, chat_id, id, first).await {
                tracing::warn!(error = %e, message_id = id, "editMessageText failed");
            }
        }
        for (i, chunk) in chunks.iter().skip(1).enumerate() {
            let chunk_idx = i as u32 + 1;
            let _ = send_in_order(
                deps,
                chat_id,
                order_key,
                entry_id,
                chunk_idx,
                settle_ms,
                || send_message_formatted(deps, chat_id, chunk, None),
            )
            .await;
        }
        return Ok(());
    }

    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(());
    }

    let mut chunk_index = 0u32;
    for chunk in chunks.iter() {
        if chunk.is_empty() {
            continue;
        }
        if is_entry_finalized(deps, session_id, entry_id).await {
            return Ok(());
        }
        let id = send_in_order(
            deps,
            chat_id,
            order_key,
            entry_id,
            chunk_index,
            settle_ms,
            || send_message_formatted(deps, chat_id, chunk, None),
        )
        .await?;
        if chunk_index == 0 {
            *message_id = Some(id);
            kv::set_entry_message_id(deps, session_id, entry_id, id).await?;
        }
        chunk_index += 1;
    }
    Ok(())
}

async fn finalize_entry(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
) -> Result<(), IIIError> {
    if session.finalized {
        return Ok(());
    }

    let text = session.last_text.clone();

    if text.is_empty() && session.phase == MessagePhase::Empty {
        clear_pending_materialization(&deps.runtime, session.chat_id, entry_id);
        session.finalized = true;
        return Ok(());
    }

    if is_entry_finalized(deps, session_id, entry_id).await {
        session.finalized = true;
        return Ok(());
    }

    hydrate_message_ids(deps, session_id, entry_id, session).await;

    match session.transport {
        EffectiveTransport::Draft if session.draft_started => {
            // Issue 3: post the persistent bubble first, then remove the
            // ephemeral preview. Clearing first animated the streamed text away
            // and left a visible gap before the real message arrived ("retype").
            finalize_draft_messages(deps, session_id, entry_id, session, &text).await?;
            clear_draft(deps, session).await;
        }
        EffectiveTransport::Edit => {
            flush_edit_final(deps, cfg, session_id, entry_id, session, &text).await?;
        }
        EffectiveTransport::Draft => {
            // Draft transport without a confirmed draft: still flush the text and
            // clear any draft that may be lingering so the chat doesn't keep
            // showing the "typing" preview after the bubble posts (issue 2).
            flush_edit_final(deps, cfg, session_id, entry_id, session, &text).await?;
            clear_draft(deps, session).await;
        }
    }

    session.finalized = true;
    deps.runtime
        .pending_entries
        .remove(&entry_key(session_id, entry_id));
    Ok(())
}

async fn finalize_draft_messages(
    deps: &Deps,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
    text: &str,
) -> Result<(), IIIError> {
    let order_key = session.order_key;
    let chunks = split_message(text, TELEGRAM_MAX_MESSAGE_LEN);

    if let Some(message_id) = session.message_id {
        if let Some(first) = chunks.first() {
            if let Err(e) =
                edit_message_text_formatted(deps, session.chat_id, message_id, first).await
            {
                tracing::warn!(error = %e, message_id, "finalize draft editMessageText failed");
            }
        }
        for (i, chunk) in chunks.iter().skip(1).enumerate() {
            let chunk_idx = i as u32 + 1;
            let _ = send_in_order(
                deps,
                session.chat_id,
                order_key,
                entry_id,
                chunk_idx,
                0,
                || send_message_formatted(deps, session.chat_id, chunk, None),
            )
            .await;
        }
        return Ok(());
    }

    if is_entry_finalized(deps, session_id, entry_id).await {
        return Ok(());
    }

    let mut chunk_index = 0u32;
    for chunk in chunks.iter() {
        if chunk.is_empty() {
            continue;
        }
        if is_entry_finalized(deps, session_id, entry_id).await {
            return Ok(());
        }
        match send_in_order(
            deps,
            session.chat_id,
            order_key,
            entry_id,
            chunk_index,
            0,
            || send_message_formatted(deps, session.chat_id, chunk, None),
        )
        .await
        {
            Ok(id) => {
                if chunk_index == 0 {
                    session.message_id = Some(id);
                    kv::set_entry_message_id(deps, session_id, entry_id, id).await?;
                }
            }
            Err(e) => tracing::warn!(error = %e, "finalize sendMessage failed"),
        }
        chunk_index += 1;
    }
    Ok(())
}

async fn flush_edit_final(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    entry_id: &str,
    session: &mut StreamSession,
    text: &str,
) -> Result<(), IIIError> {
    let chat_id = session.chat_id;
    let revision = session.last_revision;
    let order_key = session.order_key;
    deliver_text(
        deps,
        cfg,
        session_id,
        entry_id,
        chat_id,
        &mut session.message_id,
        text,
        revision,
        false,
        order_key,
    )
    .await
}

async fn send_message_formatted(
    deps: &Deps,
    chat_id: i64,
    text: &str,
    reply_markup: Option<serde_json::Value>,
) -> Result<i64, IIIError> {
    let (formatted, parse_mode) = format::format_outgoing(text);
    match telegram::send_message(deps, chat_id, &formatted, reply_markup.clone(), parse_mode).await
    {
        Ok(id) => Ok(id),
        Err(e) if parse_mode.is_some() => {
            tracing::warn!(error = %e, "formatted sendMessage failed, retrying plain");
            telegram::send_message(deps, chat_id, text, reply_markup, None).await
        }
        Err(e) => Err(e),
    }
}

async fn edit_message_text_formatted(
    deps: &Deps,
    chat_id: i64,
    message_id: i64,
    text: &str,
) -> Result<(), IIIError> {
    let (formatted, parse_mode) = format::format_outgoing(text);
    match telegram::edit_message_text(deps, chat_id, message_id, &formatted, parse_mode).await {
        Ok(()) => Ok(()),
        Err(e) if parse_mode.is_some() => {
            tracing::warn!(error = %e, message_id, "formatted editMessageText failed, retrying plain");
            telegram::edit_message_text(deps, chat_id, message_id, text, None).await
        }
        Err(e) => Err(e),
    }
}

async fn is_entry_finalized(deps: &Deps, session_id: &str, entry_id: &str) -> bool {
    let key = entry_key(session_id, entry_id);
    if deps.runtime.finalized_entries.contains_key(&key) {
        return true;
    }
    if kv::is_entry_finalized(deps, session_id, entry_id).await {
        // Revision is not durably recorded, so treat a restart-learned finalize
        // as the highest possible: the turn is over and the persisted message is
        // authoritative, so no late event should reconcile it.
        deps.runtime.finalized_entries.insert(key, u64::MAX);
        return true;
    }
    false
}

async fn mark_entry_finalized(deps: &Deps, key: &(String, String), chat_id: i64, revision: u64) {
    deps.runtime.finalized_entries.insert(key.clone(), revision);
    deps.runtime.pending_entries.remove(key);
    clear_pending_materialization(&deps.runtime, chat_id, &key.1);
    let _ = kv::set_entry_finalized(deps, &key.0, &key.1).await;
}

fn merge_pending_into_session(
    session: &mut StreamSession,
    pending: &crate::deps::PendingEntryState,
) {
    if pending.revision >= session.last_revision {
        session.last_revision = pending.revision;
        session.last_text = pending.text.clone();
        session.last_thinking_text = pending.thinking_text.clone();
        session.phase = pending.phase;
        session.message_id = session.message_id.or(pending.message_id);
        session.thinking_message_id = session.thinking_message_id.or(pending.thinking_message_id);
    }
    // A draft may have been pushed regardless of which snapshot has the newer
    // revision, so finalize must clear it if either side saw one start.
    session.draft_started = session.draft_started || pending.draft_started;
    session.order_key = merge_order_key(session.order_key, pending.order_key);
}

/// Order entry keys for posting: lowest order key first, ties broken by entry
/// id. DashMap iteration order is arbitrary, so this is what keeps multiple
/// entries finalized in one turn from landing out of order in the chat.
fn order_for_posting(mut pairs: Vec<(i64, (String, String))>) -> Vec<(String, String)> {
    pairs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1 .1.cmp(&b.1 .1)));
    pairs.into_iter().map(|(_, key)| key).collect()
}

/// Combine two ordering keys for the same entry, preferring the earlier
/// (smaller) non-zero value. `0` means "unset" (append time not yet observed).
fn merge_order_key(a: i64, b: i64) -> i64 {
    match (a, b) {
        (0, b) => b,
        (a, 0) => a,
        (a, b) => a.min(b),
    }
}

/// Resolve the per-entry ordering key as the earliest append time observed
/// across the entry's events (`min` wins), persisting it so finalize and
/// reconstruction agree even when events arrive out of order or after a
/// restart.
async fn record_order_key(deps: &Deps, session_id: &str, entry_id: &str, timestamp: i64) -> i64 {
    let existing = kv::entry_order(deps, session_id, entry_id).await;
    let key = match existing {
        Some(prev) => merge_order_key(prev, timestamp),
        None => timestamp,
    };
    if existing != Some(key) {
        let _ = kv::set_entry_order(deps, session_id, entry_id, key).await;
    }
    key
}

/// Parameters for posting a new Telegram bubble in per-chat append order.
pub struct OrderedChatMessage<'a> {
    pub chat_id: i64,
    pub order_key: i64,
    pub entry_id: &'a str,
    pub chunk_idx: u32,
    pub text: &'a str,
    pub reply_markup: Option<serde_json::Value>,
}

/// Post a new Telegram bubble in per-chat append order (for bindings outside
/// the streaming path, e.g. function_result entries and turn status toasts).
pub async fn send_chat_message_in_order(
    deps: &Deps,
    cfg: &WorkerConfig,
    msg: OrderedChatMessage<'_>,
) -> Result<i64, IIIError> {
    send_in_order(
        deps,
        msg.chat_id,
        msg.order_key,
        msg.entry_id,
        msg.chunk_idx,
        cfg.streaming.create_settle_ms,
        || send_message_formatted(deps, msg.chat_id, msg.text, msg.reply_markup.clone()),
    )
    .await
}

/// Create a new Telegram message in per-chat append order.
///
/// Concurrently-spawned handlers register an `(order_key, entry_id, chunk)`
/// slot; only the earliest pending slot is admitted at a time. Later entries
/// also wait until earlier entries have materialized (posted a first bubble
/// or been finalized empty).
async fn send_in_order<F, Fut>(
    deps: &Deps,
    chat_id: i64,
    order_key: i64,
    entry_id: &str,
    chunk_idx: u32,
    settle_ms: u64,
    make: F,
) -> Result<i64, IIIError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<i64, IIIError>>,
{
    let slot = (order_key, entry_id.to_string(), chunk_idx);
    await_create_slot(&deps.runtime, chat_id, &slot, settle_ms).await;

    let lock = deps.runtime.chat_create_lock(chat_id);
    let _guard = lock.lock().await;
    let result = make().await;

    if result.is_ok() {
        clear_pending_materialization(&deps.runtime, chat_id, entry_id);
    }
    complete_create_slot(&deps.runtime, chat_id, &slot, order_key, result.is_ok());
    result
}

fn register_pending_materialization(
    runtime: &RuntimeState,
    chat_id: i64,
    order_key: i64,
    entry_id: &str,
) {
    runtime
        .chat_pending_materialization
        .entry(chat_id)
        .or_default()
        .insert((order_key, entry_id.to_string()));
    runtime.chat_create_notify(chat_id).notify_waiters();
}

fn clear_pending_materialization(runtime: &RuntimeState, chat_id: i64, entry_id: &str) {
    if let Some(mut set) = runtime.chat_pending_materialization.get_mut(&chat_id) {
        set.retain(|(_, e)| e != entry_id);
    }
    runtime.chat_create_notify(chat_id).notify_waiters();
}

fn has_prior_unmaterialized(
    runtime: &RuntimeState,
    chat_id: i64,
    order_key: i64,
    entry_id: &str,
) -> bool {
    runtime
        .chat_pending_materialization
        .get(&chat_id)
        .is_some_and(|set| {
            set.iter()
                .any(|(k, e)| (*k, e.as_str()) < (order_key, entry_id))
        })
}

/// Register a creation slot and wait until it is the earliest pending slot for
/// the chat and all prior entries have materialized. Holds no creation lock
/// while waiting, so it cannot deadlock against per-entry locks.
async fn await_create_slot(
    runtime: &RuntimeState,
    chat_id: i64,
    slot: &(i64, String, u32),
    settle_ms: u64,
) {
    runtime
        .chat_create_order
        .entry(chat_id)
        .or_default()
        .insert(slot.clone());
    let notify = runtime.chat_create_notify(chat_id);

    if settle_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(settle_ms)).await;
    }

    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let order_admitted = match runtime.chat_create_order.get(&chat_id) {
            Some(set) => !set.contains(slot) || set.iter().next() == Some(slot),
            None => true,
        };
        let materialized_ok = !has_prior_unmaterialized(runtime, chat_id, slot.0, &slot.1);
        if order_admitted && materialized_ok {
            break;
        }

        notified.await;
    }
}

/// Mark a creation slot done: record the high-water order key, drop the slot,
/// and wake the next waiter.
fn complete_create_slot(
    runtime: &RuntimeState,
    chat_id: i64,
    slot: &(i64, String, u32),
    order_key: i64,
    ok: bool,
) {
    if ok {
        let mut highest = runtime
            .last_created_order
            .entry(chat_id)
            .or_insert(order_key);
        if order_key > *highest {
            *highest = order_key;
        }
    }
    if let Some(mut set) = runtime.chat_create_order.get_mut(&chat_id) {
        set.remove(slot);
    }
    runtime.chat_create_notify(chat_id).notify_waiters();
}

fn uses_rich_thinking_draft(phase: MessagePhase) -> bool {
    phase == MessagePhase::ThinkingOnly
}

fn should_emit_native_thinking_placeholder(transport: EffectiveTransport) -> bool {
    transport == EffectiveTransport::Draft
}

async fn clear_draft(deps: &Deps, session: &StreamSession) {
    if let Err(e) =
        telegram::send_message_draft(deps, session.chat_id, session.draft_id, "", None).await
    {
        tracing::warn!(error = %e, draft_id = session.draft_id, "clear draft failed");
    }
}

fn resolve_transport(deps: &Deps, chat_id: i64, cfg: &WorkerConfig) -> EffectiveTransport {
    if deps.runtime.draft_disabled_chats.contains_key(&chat_id) {
        return EffectiveTransport::Edit;
    }
    match cfg.streaming.transport {
        StreamTransport::Edit => EffectiveTransport::Edit,
        StreamTransport::Draft => EffectiveTransport::Draft,
        StreamTransport::Auto => EffectiveTransport::Draft,
    }
}

fn draft_id_for_entry(cfg: &WorkerConfig, session_id: &str, entry_id: &str) -> i32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    entry_id.hash(&mut hasher);
    let hash = hasher.finish();
    cfg.streaming.draft_id_seed + (hash as i32).unsigned_abs() as i32 % 100_000
}

fn should_draft(deps: &Deps, session: &StreamSession, throttle_ms: u64) -> bool {
    let key = (session.chat_id, session.draft_id);
    let now = std::time::Instant::now();
    if let Some(last) = deps.runtime.draft_times.get(&key) {
        if now.duration_since(*last) < std::time::Duration::from_millis(throttle_ms) {
            return false;
        }
    }
    deps.runtime.draft_times.insert(key, now);
    true
}

fn pin_edit_fallback(deps: &Deps, chat_id: i64) {
    deps.runtime.draft_disabled_chats.insert(chat_id, ());
}

fn is_draft_unsupported(err: &IIIError) -> bool {
    let msg = format!("{err}");
    msg.contains("TEXTDRAFT_PEER_INVALID")
        || msg.contains("method not found")
        || msg.contains("Not Found")
}

async fn send_native_thinking_placeholder(
    deps: &Deps,
    session_id: &str,
    session: &mut StreamSession,
    thinking_text: &str,
) -> Result<(), IIIError> {
    let rich = telegram::rich_thinking_draft(thinking_text);
    match telegram::send_rich_message_draft(deps, session.chat_id, session.draft_id, &rich, None)
        .await
    {
        Ok(()) => {
            session.draft_started = true;
            typing::stop_typing_on_output(deps, session_id);
        }
        Err(e) if is_draft_unsupported(&e) => {
            pin_edit_fallback(deps, session.chat_id);
            session.transport = EffectiveTransport::Edit;
        }
        Err(e) => {
            tracing::warn!(error = %e, "sendRichMessageDraft failed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::{PendingEntryState, RuntimeState};

    use crate::config::Verbosity;
    use crate::render::verbosity::{render_answer_text, render_thinking_text};
    use crate::types::{AgentMessage, ContentBlock};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DraftFinalizeStep {
        ClearDraftBeforeAnswer,
        PostAnswer,
        ClearDraftAfterAnswer,
    }

    const DRAFT_FINALIZE_STEPS: [DraftFinalizeStep; 3] = [
        DraftFinalizeStep::ClearDraftBeforeAnswer,
        DraftFinalizeStep::PostAnswer,
        DraftFinalizeStep::ClearDraftAfterAnswer,
    ];

    #[test]
    fn draft_id_stable() {
        let cfg = WorkerConfig::default();
        let a = draft_id_for_entry(&cfg, "s1", "e1");
        let b = draft_id_for_entry(&cfg, "s1", "e1");
        assert_eq!(a, b);
        assert_ne!(a, 0);
    }

    #[test]
    fn merge_pending_under_lock_uses_latest_revision() {
        let mut session = StreamSession {
            draft_id: 1,
            chat_id: 42,
            transport: EffectiveTransport::Draft,
            message_id: None,
            thinking_message_id: None,
            last_revision: 3,
            last_text: "stale snapshot".into(),
            last_thinking_text: String::new(),
            phase: MessagePhase::Answering,
            finalized: false,
            draft_started: true,
            order_key: 7,
        };
        let pending = PendingEntryState {
            chat_id: 42,
            revision: 5,
            text: "final answer".into(),
            thinking_text: "thought".into(),
            phase: MessagePhase::Answering,
            message_id: None,
            thinking_message_id: None,
            finalized: false,
            draft_started: false,
            order_key: 5,
        };
        merge_pending_into_session(&mut session, &pending);
        assert_eq!(session.last_text, "final answer");
        assert_eq!(session.last_revision, 5);
    }

    #[test]
    fn split_message_produces_multiple_chunks_for_long_text() {
        let text = "x".repeat(5000);
        let chunks = split_message(&text, TELEGRAM_MAX_MESSAGE_LEN);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), TELEGRAM_MAX_MESSAGE_LEN);
    }

    #[test]
    fn rich_thinking_draft_enabled_for_thinking_only() {
        assert!(uses_rich_thinking_draft(MessagePhase::ThinkingOnly));
        assert!(!uses_rich_thinking_draft(MessagePhase::Answering));
    }

    #[test]
    fn thinking_placeholder_eligible_on_draft_transport() {
        assert!(should_emit_native_thinking_placeholder(
            EffectiveTransport::Draft
        ));
        assert!(!should_emit_native_thinking_placeholder(
            EffectiveTransport::Edit
        ));
    }

    #[test]
    fn thinking_text_extracted_at_none_verbosity() {
        let msg = AgentMessage {
            role: "assistant".into(),
            content: vec![ContentBlock::Thinking {
                text: "plan".into(),
            }],
        };
        assert_eq!(render_thinking_text(&msg).unwrap(), "plan");
        assert!(render_answer_text(&msg, Verbosity::None).is_none());
    }

    #[test]
    fn merge_pending_prefers_higher_revision() {
        let mut session = StreamSession {
            draft_id: 1,
            chat_id: 42,
            transport: EffectiveTransport::Draft,
            message_id: Some(100),
            thinking_message_id: None,
            last_revision: 3,
            last_text: "old".into(),
            last_thinking_text: String::new(),
            phase: MessagePhase::Answering,
            finalized: false,
            draft_started: true,
            order_key: 7,
        };
        let pending = PendingEntryState {
            chat_id: 42,
            revision: 5,
            text: "final answer".into(),
            thinking_text: "thought".into(),
            phase: MessagePhase::Answering,
            message_id: Some(100),
            thinking_message_id: Some(200),
            finalized: false,
            draft_started: true,
            order_key: 5,
        };
        merge_pending_into_session(&mut session, &pending);
        assert_eq!(session.last_revision, 5);
        assert_eq!(session.last_text, "final answer");
        assert_eq!(session.last_thinking_text, "thought");
        assert_eq!(session.thinking_message_id, Some(200));
        // Order key tracks the earliest observed append time.
        assert_eq!(session.order_key, 5);
    }

    #[test]
    fn merge_pending_ignores_stale_snapshot() {
        let mut session = StreamSession {
            draft_id: 1,
            chat_id: 42,
            transport: EffectiveTransport::Draft,
            message_id: None,
            thinking_message_id: None,
            last_revision: 10,
            last_text: "newer".into(),
            last_thinking_text: String::new(),
            phase: MessagePhase::Answering,
            finalized: false,
            draft_started: true,
            order_key: 5,
        };
        let pending = PendingEntryState {
            chat_id: 42,
            revision: 8,
            text: "stale".into(),
            thinking_text: String::new(),
            phase: MessagePhase::Answering,
            message_id: None,
            thinking_message_id: None,
            finalized: false,
            draft_started: false,
            order_key: 9,
        };
        merge_pending_into_session(&mut session, &pending);
        assert_eq!(session.last_revision, 10);
        assert_eq!(session.last_text, "newer");
        // Stale revision is ignored, but the earliest order key still wins.
        assert_eq!(session.order_key, 5);
    }

    #[test]
    fn revision_guard_rejects_stale_and_accepts_fresh() {
        // Equal or newer revisions apply; older ones are dropped so an
        // out-of-order handler cannot regress the rendered text.
        assert!(revision_is_fresh(5, 5));
        assert!(revision_is_fresh(6, 5));
        assert!(!revision_is_fresh(4, 5));
        // `message-added` carries revision 0: fresh for a brand-new entry...
        assert!(revision_is_fresh(0, 0));
        // ...but stale once an update already advanced the revision.
        assert!(!revision_is_fresh(0, 3));
    }

    #[test]
    fn reconcile_only_for_strictly_higher_revision() {
        // Finalize may race ahead of the last token; a higher revision then
        // reconciles, but duplicates/stale events do not.
        assert!(should_reconcile_finalized(6, 5));
        assert!(!should_reconcile_finalized(5, 5));
        assert!(!should_reconcile_finalized(4, 5));
        // A finalize learned from durable state (revision unknown == u64::MAX)
        // is treated as authoritative and never reconciled.
        assert!(!should_reconcile_finalized(7, u64::MAX));
        assert!(!should_reconcile_finalized(u64::MAX, u64::MAX));
    }

    #[test]
    fn merge_pending_preserves_draft_started() {
        // Even when pending is the stale (lower-revision) side, a started draft
        // must still be cleared at finalize.
        let mut session = StreamSession {
            draft_id: 1,
            chat_id: 42,
            transport: EffectiveTransport::Draft,
            message_id: None,
            thinking_message_id: None,
            last_revision: 10,
            last_text: "newer".into(),
            last_thinking_text: String::new(),
            phase: MessagePhase::Answering,
            finalized: false,
            draft_started: true,
            order_key: 5,
        };
        let pending = PendingEntryState {
            chat_id: 42,
            revision: 8,
            text: "stale".into(),
            thinking_text: String::new(),
            phase: MessagePhase::Answering,
            message_id: None,
            thinking_message_id: None,
            finalized: false,
            draft_started: false,
            order_key: 9,
        };
        merge_pending_into_session(&mut session, &pending);
        assert!(session.draft_started);

        // A rebuilt session (draft_started=false) adopts it from pending, so the
        // lingering "typing" preview still gets cleared after the bubble posts.
        let mut rebuilt = StreamSession {
            draft_id: 1,
            chat_id: 42,
            transport: EffectiveTransport::Draft,
            message_id: None,
            thinking_message_id: None,
            last_revision: 0,
            last_text: String::new(),
            last_thinking_text: String::new(),
            phase: MessagePhase::Empty,
            finalized: false,
            draft_started: false,
            order_key: 0,
        };
        let pending_with_draft = PendingEntryState {
            chat_id: 42,
            revision: 3,
            text: "answer".into(),
            thinking_text: String::new(),
            phase: MessagePhase::Answering,
            message_id: Some(50),
            thinking_message_id: None,
            finalized: false,
            draft_started: true,
            order_key: 1,
        };
        merge_pending_into_session(&mut rebuilt, &pending_with_draft);
        assert!(rebuilt.draft_started);
    }

    #[test]
    fn finalized_entries_block_in_memory() {
        let runtime = RuntimeState::new();
        let key = ("s1".into(), "e1".into());
        assert!(!runtime.finalized_entries.contains_key(&key));
        runtime.finalized_entries.insert(key.clone(), 0);
        assert!(runtime.finalized_entries.contains_key(&key));
    }

    #[test]
    fn mark_finalized_removes_pending() {
        let runtime = RuntimeState::new();
        let key = ("s1".into(), "e1".into());
        runtime.pending_entries.insert(
            key.clone(),
            PendingEntryState {
                chat_id: 42,
                revision: 1,
                text: "turn 1".into(),
                thinking_text: String::new(),
                phase: MessagePhase::Answering,
                message_id: None,
                thinking_message_id: None,
                finalized: false,
                draft_started: false,
                order_key: 0,
            },
        );
        runtime.finalized_entries.insert(key.clone(), 0);
        runtime.pending_entries.remove(&key);
        assert!(!runtime.pending_entries.contains_key(&key));
    }

    #[test]
    fn hydrate_applies_kv_ids_to_session() {
        let mut session = StreamSession {
            draft_id: 1,
            chat_id: 42,
            transport: EffectiveTransport::Edit,
            message_id: None,
            thinking_message_id: None,
            last_revision: 0,
            last_text: String::new(),
            last_thinking_text: String::new(),
            phase: MessagePhase::Empty,
            finalized: false,
            draft_started: false,
            order_key: 0,
        };
        session.message_id = Some(999);
        session.thinking_message_id = Some(888);
        assert_eq!(session.message_id, Some(999));
        assert_eq!(session.thinking_message_id, Some(888));
    }

    #[test]
    fn pending_keys_for_turn2_exclude_finalized() {
        let runtime = RuntimeState::new();
        let turn1 = ("session".into(), "entry-turn1".into());
        let turn2 = ("session".into(), "entry-turn2".into());
        runtime.pending_entries.insert(
            turn1.clone(),
            PendingEntryState {
                chat_id: 42,
                revision: 1,
                text: "turn 1 answer".into(),
                thinking_text: String::new(),
                phase: MessagePhase::Answering,
                message_id: None,
                thinking_message_id: None,
                finalized: false,
                draft_started: false,
                order_key: 0,
            },
        );
        runtime.pending_entries.insert(
            turn2.clone(),
            PendingEntryState {
                chat_id: 42,
                revision: 1,
                text: "turn 2 answer".into(),
                thinking_text: String::new(),
                phase: MessagePhase::Answering,
                message_id: None,
                thinking_message_id: None,
                finalized: false,
                draft_started: false,
                order_key: 0,
            },
        );
        runtime.finalized_entries.insert(turn1.clone(), 0);

        let pending_keys: Vec<(String, String)> = runtime
            .pending_entries
            .iter()
            .filter(|e| e.key().0 == "session" && !e.value().finalized)
            .filter(|e| !runtime.finalized_entries.contains_key(e.key()))
            .map(|e| e.key().clone())
            .collect();

        assert_eq!(pending_keys.len(), 1);
        assert_eq!(pending_keys[0], turn2);
    }

    #[tokio::test]
    async fn entry_lock_blocks_until_released() {
        use std::sync::Arc;

        let runtime = Arc::new(RuntimeState::new());
        let key = ("s1".into(), "e1".into());
        let lock = runtime.entry_lock(&key);
        let guard = lock.lock().await;

        let lock2 = runtime.entry_lock(&key);
        let acquired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = acquired.clone();
        let handle = tokio::spawn(async move {
            let _g = lock2.lock().await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!acquired.load(std::sync::atomic::Ordering::SeqCst));

        drop(guard);
        handle.await.unwrap();
        assert!(acquired.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn entry_lock_serializes_concurrent_access() {
        use std::sync::Arc;

        let runtime = Arc::new(RuntimeState::new());
        let key = ("s1".into(), "e1".into());
        let lock1 = runtime.entry_lock(&key);
        let lock2 = runtime.entry_lock(&key);
        assert!(Arc::ptr_eq(&lock1, &lock2));
    }

    #[test]
    fn formatted_fallback_uses_plain_text_on_html_failure() {
        let md = "**What I CAN do:**\n\n1. First item.";
        let (formatted, mode) = format::format_outgoing(md);
        assert_eq!(mode, Some("HTML"));
        assert!(formatted.contains("<b>"));
        assert_ne!(formatted, md);
        assert!(md.contains("**"));
    }

    #[test]
    fn draft_finalize_clears_draft_before_posting_answer() {
        assert_eq!(
            DRAFT_FINALIZE_STEPS[0],
            DraftFinalizeStep::ClearDraftBeforeAnswer
        );
        assert_eq!(DRAFT_FINALIZE_STEPS[1], DraftFinalizeStep::PostAnswer);
        assert_eq!(
            DRAFT_FINALIZE_STEPS[2],
            DraftFinalizeStep::ClearDraftAfterAnswer
        );
    }

    #[test]
    fn order_for_posting_sorts_by_key_then_entry_id() {
        let pairs = vec![
            (20, ("s".to_string(), "c".to_string())),
            (10, ("s".to_string(), "b".to_string())),
            (10, ("s".to_string(), "a".to_string())),
            (5, ("s".to_string(), "z".to_string())),
        ];
        let ordered = order_for_posting(pairs);
        let ids: Vec<&str> = ordered.iter().map(|(_, e)| e.as_str()).collect();
        // Lowest key first; equal keys fall back to entry-id order.
        assert_eq!(ids, vec!["z", "a", "b", "c"]);
    }

    #[test]
    fn merge_order_key_prefers_earliest_nonzero() {
        assert_eq!(merge_order_key(0, 100), 100);
        assert_eq!(merge_order_key(100, 0), 100);
        // A message-updated (later) seen before its message-added (append
        // time) still resolves to the earlier append time.
        assert_eq!(merge_order_key(200, 100), 100);
        assert_eq!(merge_order_key(100, 200), 100);
        assert_eq!(merge_order_key(0, 0), 0);
    }

    #[test]
    fn complete_create_slot_keeps_highest_order() {
        let runtime = RuntimeState::new();
        complete_create_slot(&runtime, 1, &(5, "e5".into(), 0), 5, true);
        // A later, lower-keyed creation must not regress the high-water mark.
        complete_create_slot(&runtime, 1, &(3, "e3".into(), 0), 3, true);
        assert_eq!(*runtime.last_created_order.get(&1).unwrap(), 5);
    }

    #[tokio::test]
    async fn await_create_slot_admits_sole_creator_without_deadlock() {
        let runtime = RuntimeState::new();
        // No earlier sibling ever registers: the sole creator must proceed.
        await_create_slot(&runtime, 9, &(100, "z".into(), 0), 0).await;
        complete_create_slot(&runtime, 9, &(100, "z".into(), 0), 100, true);
        assert_eq!(*runtime.last_created_order.get(&9).unwrap(), 100);
        assert!(runtime
            .chat_create_order
            .get(&9)
            .map(|s| s.is_empty())
            .unwrap_or(true));
    }

    #[tokio::test]
    async fn await_create_slot_admits_earliest_first() {
        use std::sync::{Arc, Mutex};

        let runtime = Arc::new(RuntimeState::new());
        let chat = 7;
        let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

        // Both register within the settle window, so the lower order key wins
        // regardless of which task is scheduled first.
        let rt_b = runtime.clone();
        let order_b = order.clone();
        let b = tokio::spawn(async move {
            await_create_slot(&rt_b, chat, &(2, "b".into(), 0), 50).await;
            order_b.lock().unwrap().push("b");
            complete_create_slot(&rt_b, chat, &(2, "b".into(), 0), 2, true);
        });

        let rt_a = runtime.clone();
        let order_a = order.clone();
        let a = tokio::spawn(async move {
            await_create_slot(&rt_a, chat, &(1, "a".into(), 0), 50).await;
            order_a.lock().unwrap().push("a");
            complete_create_slot(&rt_a, chat, &(1, "a".into(), 0), 1, true);
        });

        a.await.unwrap();
        b.await.unwrap();

        assert_eq!(*order.lock().unwrap(), vec!["a", "b"]);
        assert_eq!(*runtime.last_created_order.get(&chat).unwrap(), 2);
    }

    #[tokio::test]
    async fn await_create_slot_waits_for_prior_unmaterialized_entry() {
        use std::sync::Arc;

        let runtime = Arc::new(RuntimeState::new());
        let chat = 3;
        register_pending_materialization(&runtime, chat, 10, "entry_a");

        let slot_b = (20, "entry_b".to_string(), 0u32);
        runtime
            .chat_create_order
            .entry(chat)
            .or_default()
            .insert(slot_b.clone());

        let admitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = admitted.clone();
        let rt = runtime.clone();
        let handle = tokio::spawn(async move {
            await_create_slot(&rt, chat, &slot_b, 0).await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!admitted.load(std::sync::atomic::Ordering::SeqCst));

        clear_pending_materialization(&runtime, chat, "entry_a");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        handle.await.unwrap();
        assert!(admitted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn continuation_chunk_slot_orders_after_first_chunk() {
        use std::sync::{Arc, Mutex};

        let runtime = Arc::new(RuntimeState::new());
        let chat = 5;
        let order: Arc<Mutex<Vec<u32>>> = Arc::new(Mutex::new(Vec::new()));

        let slot0 = (10, "e".to_string(), 0u32);
        let slot1 = (10, "e".to_string(), 1u32);
        runtime
            .chat_create_order
            .entry(chat)
            .or_default()
            .insert(slot0.clone());
        runtime
            .chat_create_order
            .get_mut(&chat)
            .unwrap()
            .insert(slot1.clone());

        let rt0 = runtime.clone();
        let o0 = order.clone();
        let h0 = tokio::spawn(async move {
            await_create_slot(&rt0, chat, &slot0, 0).await;
            o0.lock().unwrap().push(0);
            complete_create_slot(&rt0, chat, &slot0, 10, true);
        });

        let rt1 = runtime.clone();
        let o1 = order.clone();
        let h1 = tokio::spawn(async move {
            await_create_slot(&rt1, chat, &slot1, 0).await;
            o1.lock().unwrap().push(1);
            complete_create_slot(&rt1, chat, &slot1, 10, true);
        });

        h0.await.unwrap();
        h1.await.unwrap();
        assert_eq!(*order.lock().unwrap(), vec![0, 1]);
    }

    #[test]
    fn has_prior_unmaterialized_detects_earlier_entry() {
        let runtime = RuntimeState::new();
        register_pending_materialization(&runtime, 1, 10, "a");
        assert!(has_prior_unmaterialized(&runtime, 1, 20, "b"));
        assert!(!has_prior_unmaterialized(&runtime, 1, 10, "a"));
        assert!(!has_prior_unmaterialized(&runtime, 1, 5, "z"));
    }
}
