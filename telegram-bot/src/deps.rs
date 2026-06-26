use std::collections::BTreeSet;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use iii_sdk::trigger::Trigger;
use iii_sdk::IIIClient;
use reqwest::Client;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::approval_gate::ApprovalGateStatus;
use crate::configuration::ConfigCell;
use crate::render::stream::EffectiveTransport;
use crate::render::verbosity::MessagePhase;

pub const STATE_SCOPE: &str = "telegram-bot";

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub runtime: Arc<RuntimeState>,
    pub approval_gate: Arc<ApprovalGateStatus>,
}

impl Deps {
    pub fn new(
        iii: Arc<IIIClient>,
        config: ConfigCell,
        approval_gate: Arc<ApprovalGateStatus>,
    ) -> Self {
        Self {
            iii,
            config,
            runtime: Arc::new(RuntimeState::new()),
            approval_gate,
        }
    }

    pub async fn cfg(&self) -> Arc<crate::config::WorkerConfig> {
        self.config.read().await.clone()
    }
}

/// Latest render state for an assistant entry (used for final flush).
#[derive(Debug, Clone)]
pub struct PendingEntryState {
    pub chat_id: i64,
    pub revision: u64,
    pub text: String,
    pub thinking_text: String,
    pub phase: MessagePhase,
    pub message_id: Option<i64>,
    pub thinking_message_id: Option<i64>,
    pub finalized: bool,
    /// Whether a draft was already pushed for this entry, so finalize knows it
    /// must clear the ephemeral draft even when rebuilding from this snapshot.
    pub draft_started: bool,
    /// Per-entry ordering key (append time in ms); lower posts first.
    pub order_key: i64,
}

/// Per-entry streaming session state.
#[derive(Debug, Clone)]
pub struct StreamSession {
    pub draft_id: i32,
    pub chat_id: i64,
    pub transport: EffectiveTransport,
    pub message_id: Option<i64>,
    pub thinking_message_id: Option<i64>,
    pub last_revision: u64,
    pub last_text: String,
    pub last_thinking_text: String,
    pub phase: MessagePhase,
    pub finalized: bool,
    pub draft_started: bool,
    /// Per-entry ordering key (append time in ms); lower posts first.
    pub order_key: i64,
}

pub struct RuntimeState {
    pub http: Client,
    /// Last accepted revision per (session_id, entry_id).
    pub revisions: DashMap<(String, String), u64>,
    /// Last edit time per (chat_id, message_id).
    pub edit_times: DashMap<(i64, i64), Instant>,
    /// Last draft update time per (chat_id, draft_id).
    pub draft_times: DashMap<(i64, i32), Instant>,
    /// Latest render snapshot per entry (for turn-end flush).
    pub pending_entries: DashMap<(String, String), PendingEntryState>,
    /// Active stream sessions per entry.
    pub stream_sessions: DashMap<(String, String), StreamSession>,
    /// Entries whose turn has been finalized, mapped to the revision at which
    /// they were finalized. A later, higher-revision `message-updated` may still
    /// reconcile the persisted message; `u64::MAX` means "finalized, revision
    /// unknown" (e.g. learned from durable state after a restart).
    pub finalized_entries: DashMap<(String, String), u64>,
    /// Serializes `message-added`, `message-updated`, and finalize for one entry.
    pub entry_locks: DashMap<(String, String), Arc<Mutex<()>>>,
    /// Serializes new-message creation per chat so the Telegram message order
    /// matches entry order.
    pub chat_create_locks: DashMap<i64, Arc<Mutex<()>>>,
    /// In-flight new-message creation requests per chat, ordered by
    /// `(order_key, entry_id, chunk_index)`; the minimum is admitted first.
    pub chat_create_order: DashMap<i64, BTreeSet<(i64, String, u32)>>,
    /// Entries that have started (`message-added`) but not yet posted a first
    /// `sendMessage` — later entries must wait until prior slots materialize.
    pub chat_pending_materialization: DashMap<i64, BTreeSet<(i64, String)>>,
    /// Wakes creation waiters for a chat when the admitted set changes.
    pub chat_create_notifies: DashMap<i64, Arc<Notify>>,
    /// Highest order key already materialized into a Telegram message per chat.
    pub last_created_order: DashMap<i64, i64>,
    /// Chats where draft transport failed and edit fallback is pinned.
    pub draft_disabled_chats: DashMap<i64, ()>,
    /// FIFO message queue per chat_id.
    pub fifo_queues: DashMap<i64, Vec<String>>,
    /// Next `getUpdates` offset (last_update_id + 1).
    pub poll_offset: AtomicI64,
    /// Cancels the background poller on adapter switch or shutdown.
    pub poller_cancel: Mutex<Option<CancellationToken>>,
    /// Join handle for the background poller task.
    pub poller_handle: Mutex<Option<JoinHandle<()>>>,
    /// Engine HTTP-trigger handle for the webhook ingress route. `Some` only
    /// while the webhook adapter is active; retained so the route can be
    /// unregistered when switching back to polling (the SDK `Trigger` has no
    /// `Drop`, so it must be explicitly `take()`n and `.unregister()`ed).
    pub webhook_trigger: Mutex<Option<Trigger>>,
    /// Latest harness turn_id per session (for trace baggage on binding handlers).
    pub active_turns: DashMap<String, String>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeState {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            revisions: DashMap::new(),
            edit_times: DashMap::new(),
            draft_times: DashMap::new(),
            pending_entries: DashMap::new(),
            stream_sessions: DashMap::new(),
            finalized_entries: DashMap::new(),
            entry_locks: DashMap::new(),
            chat_create_locks: DashMap::new(),
            chat_create_order: DashMap::new(),
            chat_create_notifies: DashMap::new(),
            chat_pending_materialization: DashMap::new(),
            last_created_order: DashMap::new(),
            draft_disabled_chats: DashMap::new(),
            fifo_queues: DashMap::new(),
            poll_offset: AtomicI64::new(0),
            poller_cancel: Mutex::new(None),
            poller_handle: Mutex::new(None),
            webhook_trigger: Mutex::new(None),
            active_turns: DashMap::new(),
        }
    }

    /// Per-entry mutex so finalize and stream handlers cannot interleave.
    pub fn entry_lock(&self, key: &(String, String)) -> Arc<Mutex<()>> {
        self.entry_locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Per-chat mutex so only one new Telegram message is created at a time.
    pub fn chat_create_lock(&self, chat_id: i64) -> Arc<Mutex<()>> {
        self.chat_create_locks
            .entry(chat_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Per-chat notify used to wake new-message creation waiters.
    pub fn chat_create_notify(&self, chat_id: i64) -> Arc<Notify> {
        self.chat_create_notifies
            .entry(chat_id)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    /// Drop in-memory streaming/queue state when a chat starts a new session.
    pub fn reset_for_chat(&self, chat_id: i64, old_session_id: Option<&str>) {
        self.fifo_queues.remove(&chat_id);
        self.chat_create_order.remove(&chat_id);
        self.chat_pending_materialization.remove(&chat_id);
        self.last_created_order.remove(&chat_id);
        self.chat_create_locks.remove(&chat_id);
        if let Some((_, notify)) = self.chat_create_notifies.remove(&chat_id) {
            // Wake stragglers so they re-evaluate against the cleared state.
            notify.notify_waiters();
        }
        if let Some(sid) = old_session_id {
            self.stream_sessions.retain(|k, _| k.0 != sid);
            self.pending_entries.retain(|k, _| k.0 != sid);
            self.finalized_entries.retain(|k, _| k.0 != sid);
            self.entry_locks.retain(|k, _| k.0 != sid);
            self.revisions.retain(|k, _| k.0 != sid);
            self.active_turns.remove(sid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_for_chat_clears_fifo_and_session_state() {
        let runtime = RuntimeState::new();
        runtime.fifo_queues.insert(42, vec!["queued".into()]);
        runtime.stream_sessions.insert(
            ("old-session".into(), "entry-1".into()),
            StreamSession {
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
            },
        );
        runtime.pending_entries.insert(
            ("old-session".into(), "entry-1".into()),
            PendingEntryState {
                chat_id: 42,
                revision: 0,
                text: String::new(),
                thinking_text: String::new(),
                phase: MessagePhase::Empty,
                message_id: None,
                thinking_message_id: None,
                finalized: false,
                draft_started: false,
                order_key: 0,
            },
        );
        runtime.stream_sessions.insert(
            ("other-session".into(), "entry-2".into()),
            StreamSession {
                draft_id: 2,
                chat_id: 99,
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
            },
        );

        runtime.reset_for_chat(42, Some("old-session"));

        assert!(!runtime.active_turns.contains_key("old-session"));
        assert!(!runtime.fifo_queues.contains_key(&42));
        assert!(!runtime
            .stream_sessions
            .contains_key(&("old-session".into(), "entry-1".into())));
        assert!(runtime
            .stream_sessions
            .contains_key(&("other-session".into(), "entry-2".into())));
        assert!(!runtime
            .pending_entries
            .contains_key(&("old-session".into(), "entry-1".into())));
    }

    #[test]
    fn reset_for_chat_clears_sequencing_state() {
        let runtime = RuntimeState::new();
        let mut set = BTreeSet::new();
        set.insert((1i64, "entry-1".to_string(), 0));
        runtime.chat_create_order.insert(42, set);
        runtime
            .chat_pending_materialization
            .insert(42, BTreeSet::from([(2i64, "entry-2".to_string())]));
        runtime.last_created_order.insert(42, 5);
        let _ = runtime.chat_create_lock(42);
        let _ = runtime.chat_create_notify(42);
        // Unrelated chat must survive.
        runtime.last_created_order.insert(99, 1);

        runtime.reset_for_chat(42, None);

        assert!(!runtime.chat_create_order.contains_key(&42));
        assert!(!runtime.chat_pending_materialization.contains_key(&42));
        assert!(!runtime.last_created_order.contains_key(&42));
        assert!(!runtime.chat_create_locks.contains_key(&42));
        assert!(!runtime.chat_create_notifies.contains_key(&42));
        assert!(runtime.last_created_order.contains_key(&99));
    }
}
