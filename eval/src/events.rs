use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::EvalStatusV1;

pub const COMPLETED: &str = "eval::completed";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompletedBindingConfigV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_id: Option<String>,
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    evaluation_id: Option<String>,
    metadata: Option<Value>,
}

#[derive(Clone, Default)]
struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    fn insert(&self, config: TriggerConfig) -> Result<(), String> {
        let raw = if config.config.is_null() {
            json!({})
        } else {
            config.config
        };
        let filter: CompletedBindingConfigV1 = serde_json::from_value(raw)
            .map_err(|error| format!("invalid eval::completed config: {error}"))?;
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                config.id,
                Binding {
                    function_id: config.function_id,
                    evaluation_id: filter.evaluation_id,
                    metadata: config.metadata,
                },
            );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }
}

struct CompletedTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for CompletedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.insert(config).map_err(Error::Handler)
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

#[derive(Clone)]
pub struct EvalEvents {
    iii: Arc<IIIClient>,
    completed: SubscriberSet,
}

impl EvalEvents {
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let completed = SubscriberSet::default();
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                COMPLETED,
                "An evaluation reached a terminal status.",
                CompletedTriggerHandler {
                    set: completed.clone(),
                },
            )
            .trigger_request_format::<CompletedBindingConfigV1>(),
        );
        Self {
            iii: iii.clone(),
            completed,
        }
    }

    pub async fn emit_completed(
        &self,
        evaluation_id: &str,
        status: EvalStatusV1,
        eligible: Option<bool>,
    ) {
        let mut payload = json!({
            "evaluation_id": evaluation_id,
            "status": status,
            "timestamp": crate::ids::now_ms(),
        });
        if let Some(eligible) = eligible {
            payload["eligible"] = Value::Bool(eligible);
        }
        for binding in self.completed.snapshot() {
            if binding
                .evaluation_id
                .as_deref()
                .is_some_and(|filter| filter != evaluation_id)
            {
                continue;
            }
            let request = TriggerRequest {
                function_id: binding.function_id,
                payload: payload.clone(),
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let result = match binding.metadata {
                Some(metadata) => self.iii.trigger(request.metadata(metadata)).await,
                None => self.iii.trigger(request).await,
            };
            if let Err(error) = result {
                tracing::warn!(%evaluation_id, %error, "eval::completed delivery failed");
            }
        }
    }
}
