//! The two async orchestration trigger types the harness emits at turn
//! boundaries — `harness::turn-started` and `harness::turn-completed`
//! (harness.md § Trigger types emitted). Consumers and siblings bind these to
//! react to outcomes without polling `harness::status`.
//!
//! Delivery is fire-and-forget (`TriggerAction::Void`), at-least-once, and
//! unordered. Per-binding `config` filters (`session_id`, `parent_session_id`)
//! are evaluated here by the emitting worker. The engine replays existing
//! registrations after a restart, so the subscriber sets rebuild themselves.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::turn::ParentLink;

pub const TURN_STARTED: &str = "harness::turn-started";
pub const TURN_COMPLETED: &str = "harness::turn-completed";

/// Binding config shared by both turn-event types.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnEventBindingConfig {
    /// Only deliver events for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Only deliver sub-agent events whose parent is this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindingFilter {
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
}

impl BindingFilter {
    fn parse(raw: &Value) -> Result<Self, String> {
        let raw = if raw.is_null() {
            Value::Object(Default::default())
        } else {
            raw.clone()
        };
        let cfg: TurnEventBindingConfig =
            serde_json::from_value(raw).map_err(|e| format!("invalid turn-event config: {e}"))?;
        Ok(BindingFilter {
            session_id: cfg.session_id,
            parent_session_id: cfg.parent_session_id,
        })
    }

    fn matches(&self, session_id: &str, parent: Option<&ParentLink>) -> bool {
        if let Some(sid) = &self.session_id {
            if sid != session_id {
                return false;
            }
        }
        if let Some(psid) = &self.parent_session_id {
            match parent {
                Some(p) if &p.session_id == psid => {}
                _ => return false,
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    filter: BindingFilter,
}

#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = BindingFilter::parse(&config.config)?;
        self.lock().insert(
            config.id,
            Binding {
                function_id: config.function_id,
                filter,
            },
        );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct TurnEventTriggerHandler {
    type_id: &'static str,
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for TurnEventTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = self.type_id, %id, %function_id, "turn-event subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// The harness's emitted turn-event subscriber sets + the engine handle for
/// fan-out. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct TurnEvents {
    iii: Arc<IIIClient>,
    started: SubscriberSet,
    completed: SubscriberSet,
}

impl TurnEvents {
    /// Register both trigger types and return the emitter. Must run before
    /// function registration so the handlers capture the subscriber sets.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let started = SubscriberSet::default();
        let completed = SubscriberSet::default();

        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TURN_STARTED,
                "A harness turn began executing (first loop step).",
                TurnEventTriggerHandler {
                    type_id: TURN_STARTED,
                    set: started.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TURN_COMPLETED,
                "A harness turn reached a terminal status (completed/cancelled/failed).",
                TurnEventTriggerHandler {
                    type_id: TURN_COMPLETED,
                    set: completed.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        tracing::info!("registered harness::turn-started / harness::turn-completed trigger types");

        Self {
            iii: iii.clone(),
            started,
            completed,
        }
    }

    pub async fn emit_started(&self, session_id: &str, turn_id: &str, parent: Option<&ParentLink>) {
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "timestamp": now_ms(),
        });
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        self.fan_out(&self.started, TURN_STARTED, session_id, parent, payload)
            .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn emit_completed(
        &self,
        session_id: &str,
        turn_id: &str,
        status: &str,
        result: Option<&Value>,
        result_error: Option<&str>,
        reason: Option<&str>,
        parent: Option<&ParentLink>,
    ) {
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "status": status,
            "timestamp": now_ms(),
        });
        if let Some(r) = result {
            payload["result"] = r.clone();
        }
        if let Some(e) = result_error {
            payload["result_error"] = Value::String(e.to_string());
        }
        if let Some(r) = reason {
            payload["reason"] = Value::String(r.to_string());
        }
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        self.fan_out(&self.completed, TURN_COMPLETED, session_id, parent, payload)
            .await;
    }

    async fn fan_out(
        &self,
        set: &SubscriberSet,
        trigger_type: &str,
        session_id: &str,
        parent: Option<&ParentLink>,
        payload: Value,
    ) {
        for binding in set.snapshot() {
            if !binding.filter.matches(session_id, parent) {
                continue;
            }
            let res = self
                .iii
                .trigger(TriggerRequest {
                    function_id: binding.function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(trigger_type, function_id = %binding.function_id, error = %e, "turn-event fan-out failed");
            }
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filter_matches_session_and_parent() {
        let f = BindingFilter {
            session_id: Some("s_1".into()),
            parent_session_id: None,
        };
        assert!(f.matches("s_1", None));
        assert!(!f.matches("s_2", None));

        let pf = BindingFilter {
            session_id: None,
            parent_session_id: Some("p_1".into()),
        };
        let parent = ParentLink {
            session_id: "p_1".into(),
            turn_id: "t".into(),
            function_call_id: "fc".into(),
        };
        assert!(pf.matches("child", Some(&parent)));
        assert!(!pf.matches("child", None));
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        assert!(BindingFilter::parse(&json!({ "sesion_id": "x" })).is_err());
        assert_eq!(
            BindingFilter::parse(&Value::Null).unwrap(),
            BindingFilter::default()
        );
    }
}
