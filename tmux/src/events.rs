//! `tmux::changed` — the live feed behind the console page.
//!
//! A binding receives the full [`Overview`] right after it registers and
//! again whenever anything in it changes. The poll loop only runs while at
//! least one binding exists, and mutations through this worker poke it so
//! their effect shows at once instead of on the next tick.
//!
//! Delivery mirrors the ide's trigger types: `TriggerAction::Void`, one task
//! per fire, and the binding's `metadata` and `namespace` passed through — a
//! fire that drops `__binding` is discarded by the harness without an error
//! (MOT-4719).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::collector::Collector;
use crate::model::Overview;

pub const CHANGED: &str = "tmux::changed";

/// `tmux::changed` takes no options.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangedConfig {}

#[derive(Debug, Clone)]
struct Subscriber {
    function_id: String,
    metadata: Option<Value>,
    namespace: Option<String>,
}

impl Subscriber {
    fn request(&self, payload: Value) -> TriggerRequestWithMetadata {
        let mut request: TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id.clone(),
            payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(metadata) = &self.metadata {
            request = request.metadata(metadata.clone());
        }
        if let Some(namespace) = &self.namespace {
            request = request.namespace(namespace.clone());
        }
        request
    }
}

/// Bindings plus the poll loop that feeds them.
pub struct Monitor {
    iii: IIIClient,
    collector: Arc<Collector>,
    subscribers: Mutex<HashMap<String, Subscriber>>,
    wake: Notify,
}

/// The overview without its timestamps, so an unchanged state is not re-sent.
fn fingerprint(overview: &Overview) -> Option<Value> {
    let mut value = serde_json::to_value(overview).ok()?;
    value["sessions"]["generated_at_ms"] = Value::Null;
    value["ports"]["generated_at_ms"] = Value::Null;
    Some(value)
}

impl Monitor {
    pub fn new(iii: IIIClient, collector: Arc<Collector>) -> Arc<Self> {
        Arc::new(Self {
            iii,
            collector,
            subscribers: Mutex::new(HashMap::new()),
            wake: Notify::new(),
        })
    }

    /// Re-collect now (after a mutation) instead of on the next tick.
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    fn targets(&self) -> Vec<Subscriber> {
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn deliver(&self, targets: Vec<Subscriber>, payload: Value) {
        for subscriber in targets {
            let iii = self.iii.clone();
            let request = subscriber.request(payload.clone());
            tokio::spawn(async move {
                if let Err(error) = iii.trigger(request).await {
                    tracing::debug!(function_id = %subscriber.function_id, %error, "tmux::changed delivery failed");
                }
            });
        }
    }

    /// The poll loop. Sleeps while nothing is bound.
    pub fn spawn(self: &Arc<Self>) {
        let monitor = self.clone();
        tokio::spawn(async move {
            let mut last: Option<Value> = None;
            loop {
                let wake = monitor.wake.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                if monitor.targets().is_empty() {
                    last = None;
                    wake.await;
                    continue;
                }
                let overview = monitor.collector.overview().await;
                let print = fingerprint(&overview);
                if print.is_some() && print != last {
                    if let Ok(payload) = serde_json::to_value(&overview) {
                        monitor.deliver(monitor.targets(), payload);
                    }
                    last = print;
                }
                let interval = Duration::from_millis(monitor.collector.config().poll_interval_ms);
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = wake => {}
                }
            }
        });
    }
}

struct ChangedHandler {
    monitor: Arc<Monitor>,
}

#[async_trait]
impl TriggerHandler for ChangedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let subscriber = Subscriber {
            function_id: config.function_id.clone(),
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
        };
        tracing::debug!(trigger_type = CHANGED, id = %config.id, "live feed bound");
        self.monitor
            .subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), subscriber.clone());
        // The first frame goes to the new binding alone, right away.
        let monitor = self.monitor.clone();
        tokio::spawn(async move {
            let overview = monitor.collector.overview().await;
            if let Ok(payload) = serde_json::to_value(&overview) {
                monitor.deliver(vec![subscriber], payload);
            }
            monitor.poke();
        });
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::debug!(trigger_type = CHANGED, id = %config.id, "live feed unbound");
        self.monitor
            .subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

pub fn register_changed_trigger(iii: &IIIClient, monitor: Arc<Monitor>) {
    // `register_trigger_type` is deprecated in iii-sdk 0.25. Kept until the
    // trigger type is retired; old consumers bind it on 0.24 and 0.25 engines.
    #[allow(deprecated)]
    let _handle = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGED,
            "Live tmux overview: fires the full state (sessions, windows, panes with agent/compose/port \
             status, compose stacks and listening ports) when bound and again whenever it changes. \
             Config takes no options.",
            ChangedHandler { monitor },
        )
        .trigger_request_format::<ChangedConfig>()
        .call_request_format::<Overview>(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PortsSnapshot, SessionsSnapshot, TmuxInfo};

    fn overview(at: u64) -> Overview {
        Overview {
            sessions: SessionsSnapshot {
                tmux: TmuxInfo::default(),
                sessions: vec![],
                generated_at_ms: at,
            },
            ports: PortsSnapshot {
                supported: true,
                error: None,
                composes: vec![],
                listeners: vec![],
                generated_at_ms: at,
            },
        }
    }

    #[test]
    fn timestamps_alone_are_not_a_change() {
        assert_eq!(fingerprint(&overview(1)), fingerprint(&overview(2)));
        let mut changed = overview(1);
        changed.ports.supported = false;
        assert_ne!(fingerprint(&overview(1)), fingerprint(&changed));
    }
}
