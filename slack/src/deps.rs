use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use iii_sdk::IIIClient;
use reqwest::Client;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock};

use crate::approval_gate::ApprovalGateStatus;
use crate::configuration::ConfigCell;

pub const STATE_SCOPE: &str = "slack";

/// Cached Slack identity, discovered via `auth.test` at boot.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Identity {
    pub team: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub bot_id: Option<String>,
    pub url: Option<String>,
    pub enterprise_id: Option<String>,
}

/// In-flight native stream for one harness turn.
#[derive(Debug, Clone, Default)]
pub struct StreamState {
    pub channel: String,
    pub thread_ts: String,
    /// Recipient user — required (with `recipient_team_id`) when streaming into a
    /// channel; omitted for DMs.
    pub recipient_user_id: Option<String>,
    /// Recipient team — required alongside `recipient_user_id` for channel streams.
    pub recipient_team_id: Option<String>,
    /// Whether the target is a DM (no recipients sent).
    pub is_dm: bool,
    /// ts of the streaming message once `chat.startStream` has run.
    pub ts: Option<String>,
    /// Full assistant text already sent, to compute the next append delta.
    pub last_text: String,
    /// Highest `message-updated` revision applied (drop older/stale revisions).
    pub last_revision: u64,
}

#[derive(Default)]
pub struct RuntimeState {
    /// session_id -> active stream.
    pub streams: DashMap<String, StreamState>,
    /// Per-session lock serializing the stream read-modify-write across awaits.
    pub stream_locks: DashMap<String, Arc<Mutex<()>>>,
    /// Seen Slack `event_id`s -> first-seen time (retry dedupe, time-windowed).
    pub seen_events: DashMap<String, Instant>,
}

impl RuntimeState {
    /// Per-session mutex so concurrent `message-*`/finalize handlers cannot
    /// interleave the stream state (mirrors telegram-bot's entry locks).
    pub fn stream_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        self.stream_locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub http: Client,
    pub identity: Arc<RwLock<Option<Identity>>>,
    pub runtime: Arc<RuntimeState>,
    pub approval_gate: Arc<ApprovalGateStatus>,
    /// Engine HTTP-trigger handles for the bridge ingress routes (Some while the
    /// HTTP bridge is enabled). Held so they can be unregistered on a config change.
    pub bridge_triggers: Arc<Mutex<Vec<iii_sdk::trigger::Trigger>>>,
    /// Cancellation token for the Socket Mode loop (Some while socket mode runs).
    pub socket_cancel: Arc<Mutex<Option<tokio_util::sync::CancellationToken>>>,
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
            http: Client::new(),
            identity: Arc::new(RwLock::new(None)),
            runtime: Arc::new(RuntimeState::default()),
            approval_gate,
            bridge_triggers: Arc::new(Mutex::new(Vec::new())),
            socket_cancel: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn cfg(&self) -> Arc<crate::config::WorkerConfig> {
        self.config.read().await.clone()
    }

    /// The bot's own user id (to detect self-authored messages / mentions).
    pub async fn bot_user_id(&self) -> Option<String> {
        self.identity
            .read()
            .await
            .as_ref()
            .and_then(|i| i.user_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_lock_is_per_session() {
        let rt = RuntimeState::default();
        let a1 = rt.stream_lock("s1");
        let a2 = rt.stream_lock("s1");
        let b = rt.stream_lock("s2");
        // Same session -> same underlying mutex; different session -> different.
        assert!(Arc::ptr_eq(&a1, &a2));
        assert!(!Arc::ptr_eq(&a1, &b));
    }

    #[tokio::test]
    async fn stream_lock_serializes_same_session() {
        let rt = RuntimeState::default();
        let lock = rt.stream_lock("s1");
        let held = lock.lock().await;
        // A second acquisition of the same session lock cannot proceed while held.
        assert!(rt.stream_lock("s1").try_lock().is_err());
        drop(held);
        // A different session is independent.
        assert!(rt.stream_lock("s2").try_lock().is_ok());
    }
}
