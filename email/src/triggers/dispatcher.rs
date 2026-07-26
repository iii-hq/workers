//! Fan-out a normalized `Event` to every subscriber registered for its
//! (account, folder). Mirrors `storage::triggers::dispatcher`.

use async_trait::async_trait;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use super::registry::TriggerRegistry;
use super::Event;

#[async_trait]
pub trait EventDispatcher: Send + Sync {
    async fn dispatch(&self, event: Event);
}

pub struct EngineDispatcher {
    iii: Arc<IIIClient>,
    registry: Arc<TriggerRegistry>,
}

impl EngineDispatcher {
    pub fn new(iii: Arc<IIIClient>, registry: Arc<TriggerRegistry>) -> Self {
        Self { iii, registry }
    }
}

#[async_trait]
impl EventDispatcher for EngineDispatcher {
    async fn dispatch(&self, event: Event) {
        let subs = self.registry.subscribers_for(&event.account, &event.folder);
        if subs.is_empty() {
            return;
        }
        let payload = json!({
            "account": event.account,
            "folder": event.folder,
            "uid": event.uid,
            "message_id": event.message_id,
            "from": event.from,
            "subject": event.subject,
            "snippet": event.snippet,
            "ts": event.ts.map(|t| t.to_rfc3339()),
        });
        for sub in subs {
            let req = TriggerRequest {
                function_id: sub.function_id.clone(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(sub.handler_timeout_ms),
            };
            let bound = Duration::from_millis(sub.handler_timeout_ms.saturating_add(1_000));
            let route_ns = self.iii.namespace().filter(|_| {
                !matches!(
                    sub.function_id.split("::").next(),
                    Some(
                        "state"
                            | "stream"
                            | "queue"
                            | "pubsub"
                            | "configuration"
                            | "cron"
                            | "http"
                            | "engine"
                            | "sandbox"
                            | "log"
                            | "secret"
                            | "kv"
                            | "iii"
                    )
                )
            });
            let result = match route_ns {
                Some(ns) => timeout(bound, self.iii.trigger(req.namespace(ns))).await,
                None => timeout(bound, self.iii.trigger(req)).await,
            };
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::warn!(
                        function_id = %sub.function_id,
                        error = %e,
                        "email::new-mail subscriber returned error"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        function_id = %sub.function_id,
                        timeout_ms = sub.handler_timeout_ms,
                        "email::new-mail subscriber timed out"
                    );
                }
            }
        }
    }
}
