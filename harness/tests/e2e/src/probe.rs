//! SDK-facing controlled function and event-driven completion observer.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

use crate::client::Client;
use crate::deadline::Deadline;
use crate::types::probe::{ControlledTargetV1, LifecycleAcceptedV1, LifecycleEventV1};
use crate::types::trace::strip_engine_fields;

pub(crate) const LIFECYCLE_FUNCTION_ID: &str = "integration-probe::turn-completed";
const LIFECYCLE_TRIGGER_TYPE: &str = "harness::turn-completed";

pub struct ScenarioProbe {
    client: Client,
    completions: Arc<Mutex<Vec<LifecycleEventV1>>>,
    completion_notify: Arc<tokio::sync::Notify>,
}

impl ScenarioProbe {
    pub async fn start(ws_url: &str) -> anyhow::Result<Self> {
        let probe = Self {
            client: Client::connect(ws_url, "integration-probe"),
            completions: Arc::new(Mutex::new(Vec::new())),
            completion_notify: Arc::new(tokio::sync::Notify::new()),
        };
        probe.register_completion_sink();
        Ok(probe)
    }

    fn register_completion_sink(&self) {
        let completions = Arc::clone(&self.completions);
        let completion_notify = Arc::clone(&self.completion_notify);
        self.client.inner().register_function(
            LIFECYCLE_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let completions = Arc::clone(&completions);
                let completion_notify = Arc::clone(&completion_notify);
                async move {
                    let event = parse_lifecycle_payload(payload).map_err(Error::Handler)?;
                    completions
                        .lock()
                        .map_err(|_| Error::Handler("integration/completion_lock_poisoned".into()))?
                        .push(event);
                    completion_notify.notify_waiters();
                    serde_json::to_value(LifecycleAcceptedV1 { accepted: true }).map_err(|error| {
                        Error::Handler(format!(
                            "integration/completion_response_serialize: {error}"
                        ))
                    })
                }
            })
            .description("Completion signal for harness integration scenarios.")
            .request_format(schema_for::<LifecycleEventV1>())
            .response_format(schema_for::<LifecycleAcceptedV1>()),
        );
    }

    pub fn register_target(
        &self,
        run_id: &str,
        target: Option<&ControlledTargetV1>,
    ) -> anyhow::Result<()> {
        let Some(target) = target else {
            return Ok(());
        };
        let prefix = format!("{run_id}::");
        anyhow::ensure!(
            target.function_id.starts_with(&prefix),
            "integration/target_scope: {} must be prefixed by {prefix}",
            target.function_id
        );
        register_controlled_function(self.client.inner(), target);
        Ok(())
    }

    pub async fn bind_completion(&self, session_id: &str) -> anyhow::Result<()> {
        self.client
            .inner()
            .register_trigger(RegisterTriggerInput {
                trigger_type: LIFECYCLE_TRIGGER_TYPE.to_string(),
                function_id: LIFECYCLE_FUNCTION_ID.to_string(),
                config: json!({ "session_id": session_id }),
                metadata: None,
            })
            .map_err(|error| {
                anyhow::anyhow!(
                    "binding {LIFECYCLE_TRIGGER_TYPE} -> {LIFECYCLE_FUNCTION_ID} failed: {error}"
                )
            })?;
        Ok(())
    }

    pub async fn wait_for_completion(
        &self,
        deadline: Deadline,
    ) -> anyhow::Result<LifecycleEventV1> {
        self.wait_for_completion_turns(1, deadline)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("completion wait returned no events"))
    }

    pub async fn wait_for_completion_turns(
        &self,
        expected: usize,
        deadline: Deadline,
    ) -> anyhow::Result<Vec<LifecycleEventV1>> {
        anyhow::ensure!(expected > 0, "expected completion count must be positive");
        loop {
            let notified = self.completion_notify.notified();
            let events = self
                .completions
                .lock()
                .map_err(|_| anyhow::anyhow!("completion observer lock poisoned"))?
                .clone();
            let turns = events
                .iter()
                .filter(|event| event.terminal)
                .map(|event| event.turn_id.as_str())
                .collect::<BTreeSet<_>>();
            if turns.len() >= expected {
                return Ok(events);
            }
            deadline
                .timeout("terminal turn completion", notified)
                .await?;
        }
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

fn parse_lifecycle_payload(mut payload: Value) -> Result<LifecycleEventV1, String> {
    strip_engine_fields(&mut payload);
    serde_json::from_value(payload)
        .map_err(|error| format!("integration/lifecycle_contract: {error}"))
}

fn register_controlled_function(iii: &iii_sdk::IIIClient, target: &ControlledTargetV1) {
    let response = target.response.clone();
    iii.register_function(
        &target.function_id,
        RegisterFunction::new_async(move |_payload: Value| {
            let response = response.clone();
            async move { Ok::<Value, Error>(response) }
        })
        .description(target.description.clone())
        .request_format(Value::Object(target.request_schema.clone()))
        .response_format(const_response_schema(&target.response)),
    );
}

fn const_response_schema(response: &Value) -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "const": response
    })
}

fn schema_for<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("JSON schema serializes")
}

#[cfg(test)]
mod tests {
    use crate::types::probe::LifecycleStatusV1;

    use super::*;

    #[test]
    fn lifecycle_parser_removes_engine_fields_and_keeps_strict_shape() {
        let event = parse_lifecycle_payload(json!({
            "session_id": "session-1",
            "turn_id": "turn-1",
            "status": "completed",
            "terminal": true,
            "timestamp": 1,
            "_caller_worker_id": "engine"
        }))
        .unwrap();
        assert_eq!(event.status, LifecycleStatusV1::Completed);
        assert!(event.terminal);

        let error = parse_lifecycle_payload(json!({
            "session_id": "session-1",
            "turn_id": "turn-1",
            "status": "completed",
            "terminal": true,
            "timestamp": 1,
            "unexpected": true
        }))
        .unwrap_err();
        assert!(error.contains("unknown field"), "{error}");
    }
}
