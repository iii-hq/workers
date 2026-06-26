use std::sync::Arc;

use iii_sdk::IIIClient;
use reqwest::Client;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::configuration::ConfigCell;

/// Cached Slack identity, discovered via `auth.test` at boot (and on demand by
/// `slack::config-status`). Never configured.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Identity {
    pub team: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub bot_id: Option<String>,
    pub url: Option<String>,
    pub enterprise_id: Option<String>,
}

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub http: Client,
    pub identity: Arc<RwLock<Option<Identity>>>,
}

impl Deps {
    pub fn new(iii: Arc<IIIClient>, config: ConfigCell) -> Self {
        Self {
            iii,
            config,
            http: Client::new(),
            identity: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn cfg(&self) -> Arc<crate::config::WorkerConfig> {
        self.config.read().await.clone()
    }
}
