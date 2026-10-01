use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use iii_sdk::{
    Error, IIIClient, RegisterTriggerType,
    protocol::TriggerRequest,
    trigger::{TriggerConfig, TriggerHandler},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{RwLock, broadcast},
    task::JoinHandle,
};

use crate::sessions::{Event, SessionManager};

pub const EVENTS: &str = "whatsapp::events";

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EventFilter {
    pub session_id: Option<String>,
    /// Empty means all event kinds; values are matched exactly.
    pub kinds: Vec<String>,
}

impl EventFilter {
    pub fn matches(&self, event: &Event) -> bool {
        self.session_id
            .as_ref()
            .is_none_or(|id| id == &event.session_id)
            && (self.kinds.is_empty() || self.kinds.contains(&event.kind))
    }

    fn validate(&self) -> Result<(), Error> {
        if self.session_id.as_ref().is_some_and(|id| {
            id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        }) {
            return Err(Error::Handler("invalid session_id filter".into()));
        }
        if self.kinds.len() > 128
            || self.kinds.iter().any(|kind| {
                kind.is_empty()
                    || kind.len() > 128
                    || !kind
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_')
            })
        {
            return Err(Error::Handler("invalid event kinds filter".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct EventRouter {
    bindings: Arc<RwLock<BTreeMap<String, (EventFilter, TriggerConfig)>>>,
}

impl EventRouter {
    pub async fn bindings_for(&self, event: &Event) -> Vec<TriggerConfig> {
        self.bindings
            .read()
            .await
            .values()
            .filter(|(filter, _)| filter.matches(event))
            .map(|(_, binding)| binding.clone())
            .collect()
    }

    async fn dispatch(&self, iii: &IIIClient, event: &Event) {
        for binding in self.bindings_for(event).await {
            let payload = match serde_json::to_value(event) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::error!(%error, "encoding WhatsApp event");
                    continue;
                }
            };
            let mut request = TriggerRequest {
                function_id: binding.function_id,
                payload,
                action: None,
                timeout_ms: Some(5000),
            }
            .namespace(binding.namespace.as_deref().unwrap_or("default"));
            if let Some(metadata) = binding.metadata {
                request = request.metadata(metadata);
            }
            match tokio::time::timeout(Duration::from_secs(5), iii.trigger(request)).await {
                Ok(Ok(_)) => {}
                _ => {
                    tracing::warn!(trigger_id = %binding.id, sequence = event.sequence, "WhatsApp event delivery failed; reconcile with events::poll")
                }
            }
        }
    }
}

#[async_trait]
impl TriggerHandler for EventRouter {
    async fn register_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        let filter: EventFilter = serde_json::from_value(binding.config.clone())?;
        filter.validate()?;
        if binding
            .namespace
            .as_deref()
            .is_some_and(|namespace| namespace.trim().is_empty())
        {
            return Err(Error::Handler("empty namespace".into()));
        }
        let mut bindings = self.bindings.write().await;
        if bindings.len() >= 1024 && !bindings.contains_key(&binding.id) {
            return Err(Error::Handler("subscription limit reached".into()));
        }
        bindings.insert(binding.id.clone(), (filter, binding));
        Ok(())
    }

    async fn unregister_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        self.bindings.write().await.remove(&binding.id);
        Ok(())
    }
}

pub fn register(iii: Arc<IIIClient>, manager: Arc<SessionManager>) -> JoinHandle<()> {
    let router = EventRouter::default();
    let mut events = manager.subscribe();
    iii.register_trigger_type(RegisterTriggerType::new(EVENTS, "WhatsApp events; bounded, in-memory, best-effort delivery. Use events::poll to reconcile gaps.", router.clone())
        .trigger_request_format::<EventFilter>().call_request_format::<Event>());
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => router.dispatch(&iii, &event).await,
                Err(broadcast::error::RecvError::Lagged(count)) => tracing::warn!(
                    count,
                    "WhatsApp event consumer lagged; reconcile with events::poll"
                ),
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
