use std::sync::Arc;

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
    /// Recipient user (required when streaming into a channel, not a DM).
    pub recipient_user_id: Option<String>,
    /// ts of the streaming message once `chat.startStream` has run.
    pub ts: Option<String>,
    /// Full assistant text already sent, to compute the next append delta.
    pub last_text: String,
}

#[derive(Default)]
pub struct RuntimeState {
    /// session_id -> active stream.
    pub streams: DashMap<String, StreamState>,
    /// Seen Slack `event_id`s (retry dedupe).
    pub seen_events: DashMap<String, ()>,
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
    /// bridge is enabled). Held so they can be unregistered on a config change.
    pub bridge_triggers: Arc<Mutex<Vec<iii_sdk::trigger::Trigger>>>,
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
