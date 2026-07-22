//! SDK-facing controlled functions and event-driven scenario observers.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::types::probe::{ControlledTargetV1, LifecycleAcceptedV1, LifecycleEventV1};
use crate::types::trace::strip_engine_fields;

pub(crate) const LIFECYCLE_FUNCTION_ID: &str = "integration-probe::turn-completed";
const READY_FUNCTION_ID: &str = "integration-probe::harness-ready";
const FUNCTIONS_FUNCTION_ID: &str = "integration-probe::functions-changed";
const TRACES_FUNCTION_ID: &str = "integration-probe::traces-changed";

const LIFECYCLE_TRIGGER_TYPE: &str = "harness::turn-completed";
const READY_TRIGGER_TYPE: &str = "harness::ready";
const FUNCTIONS_TRIGGER_TYPE: &str = "engine::functions-available";
const TRACE_TRIGGER_TYPE: &str = "trace";
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct CompletionObservation {
    pub event: LifecycleEventV1,
    pub trace_generation: u64,
}

#[derive(Clone)]
struct BoundTrigger {
    id: String,
    trigger_type: &'static str,
}

#[derive(Deserialize)]
struct RegisterTriggerResponse {
    id: String,
}

#[derive(Deserialize)]
struct FunctionsChangedPayload {
    event: String,
    functions: Vec<FunctionSummary>,
}

#[derive(Deserialize)]
struct FunctionSummary {
    function_id: String,
}

pub struct ScenarioProbe {
    client: Client,
    completions: Arc<Mutex<Vec<CompletionObservation>>>,
    completion_notify: Arc<tokio::sync::Notify>,
    ready: Arc<AtomicBool>,
    ready_notify: Arc<tokio::sync::Notify>,
    functions: Arc<Mutex<BTreeSet<String>>>,
    functions_notify: Arc<tokio::sync::Notify>,
    trace_generation: Arc<AtomicU64>,
    trace_notify: Arc<tokio::sync::Notify>,
    bindings: Mutex<Vec<BoundTrigger>>,
}

impl ScenarioProbe {
    pub async fn start(ws_url: &str) -> anyhow::Result<Self> {
        let probe = Self {
            client: Client::connect(ws_url, "integration-probe"),
            completions: Arc::new(Mutex::new(Vec::new())),
            completion_notify: Arc::new(tokio::sync::Notify::new()),
            ready: Arc::new(AtomicBool::new(false)),
            ready_notify: Arc::new(tokio::sync::Notify::new()),
            functions: Arc::new(Mutex::new(BTreeSet::new())),
            functions_notify: Arc::new(tokio::sync::Notify::new()),
            trace_generation: Arc::new(AtomicU64::new(0)),
            trace_notify: Arc::new(tokio::sync::Notify::new()),
            bindings: Mutex::new(Vec::new()),
        };
        probe.register_sinks();
        Ok(probe)
    }

    fn register_sinks(&self) {
        self.register_completion_sink();
        self.register_ready_sink();
        self.register_functions_sink();
        self.register_trace_sink();
    }

    fn register_completion_sink(&self) {
        let completions = Arc::clone(&self.completions);
        let completion_notify = Arc::clone(&self.completion_notify);
        let trace_generation = Arc::clone(&self.trace_generation);
        self.client.inner().register_function(
            LIFECYCLE_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let completions = Arc::clone(&completions);
                let completion_notify = Arc::clone(&completion_notify);
                let trace_generation = Arc::clone(&trace_generation);
                async move {
                    let event = parse_lifecycle_payload(payload).map_err(Error::Handler)?;
                    let observation = CompletionObservation {
                        event,
                        trace_generation: trace_generation.load(Ordering::Acquire),
                    };
                    completions
                        .lock()
                        .map_err(|_| Error::Handler("integration/completion_lock_poisoned".into()))?
                        .push(observation);
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

    fn register_ready_sink(&self) {
        let ready = Arc::clone(&self.ready);
        let ready_notify = Arc::clone(&self.ready_notify);
        self.client.inner().register_function(
            READY_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let ready = Arc::clone(&ready);
                let ready_notify = Arc::clone(&ready_notify);
                async move {
                    parse_ready_payload(&payload).map_err(Error::Handler)?;
                    ready.store(true, Ordering::Release);
                    ready_notify.notify_waiters();
                    Ok::<Value, Error>(json!({ "accepted": true }))
                }
            })
            .description("Readiness signal for harness integration scenarios."),
        );
    }

    fn register_functions_sink(&self) {
        let functions = Arc::clone(&self.functions);
        let functions_notify = Arc::clone(&self.functions_notify);
        self.client.inner().register_function(
            FUNCTIONS_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let functions = Arc::clone(&functions);
                let functions_notify = Arc::clone(&functions_notify);
                async move {
                    let ids = parse_function_ids(payload).map_err(Error::Handler)?;
                    *functions.lock().map_err(|_| {
                        Error::Handler("integration/functions_lock_poisoned".into())
                    })? = ids;
                    functions_notify.notify_waiters();
                    Ok::<Value, Error>(json!({ "accepted": true }))
                }
            })
            .description("Function-registry signal for harness integration scenarios."),
        );
    }

    fn register_trace_sink(&self) {
        let trace_generation = Arc::clone(&self.trace_generation);
        let trace_notify = Arc::clone(&self.trace_notify);
        self.client.inner().register_function(
            TRACES_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let trace_generation = Arc::clone(&trace_generation);
                let trace_notify = Arc::clone(&trace_notify);
                async move {
                    parse_trace_payload(&payload).map_err(Error::Handler)?;
                    trace_generation.fetch_add(1, Ordering::AcqRel);
                    trace_notify.notify_waiters();
                    Ok::<Value, Error>(json!({ "accepted": true }))
                }
            })
            .description("Trace-store change signal for harness integration scenarios."),
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

    /// Bind every observer through acknowledged engine RPCs. Because these
    /// calls use the probe connection, each response is also a barrier for the
    /// function registrations queued before it.
    pub async fn bind_observers(&self, session_id: &str, deadline: Deadline) -> anyhow::Result<()> {
        self.bind_engine_trigger(
            FUNCTIONS_TRIGGER_TYPE,
            FUNCTIONS_FUNCTION_ID,
            json!({}),
            deadline,
        )
        .await?;
        self.bind_engine_trigger(TRACE_TRIGGER_TYPE, TRACES_FUNCTION_ID, json!({}), deadline)
            .await?;
        self.bind_engine_trigger(
            LIFECYCLE_TRIGGER_TYPE,
            LIFECYCLE_FUNCTION_ID,
            json!({ "session_id": session_id }),
            deadline,
        )
        .await?;
        self.bind_engine_trigger(READY_TRIGGER_TYPE, READY_FUNCTION_ID, json!({}), deadline)
            .await?;
        Ok(())
    }

    /// Deferred trigger replay cannot await the provider SDK's registration
    /// acknowledgement because replay runs on that provider's read loop. Once
    /// readiness proves the custom types exist, replace the deferred lifecycle
    /// binding with an acknowledged active registration before any stimulus.
    pub async fn confirm_completion_binding(
        &self,
        session_id: &str,
        deadline: Deadline,
    ) -> anyhow::Result<()> {
        let active_id = self
            .bind_engine_trigger(
                LIFECYCLE_TRIGGER_TYPE,
                LIFECYCLE_FUNCTION_ID,
                json!({ "session_id": session_id }),
                deadline,
            )
            .await?;
        let stale = {
            let mut bindings = self
                .bindings
                .lock()
                .map_err(|_| anyhow::anyhow!("integration/bindings_lock_poisoned"))?;
            let stale = bindings
                .iter()
                .filter(|binding| {
                    binding.trigger_type == LIFECYCLE_TRIGGER_TYPE && binding.id != active_id
                })
                .cloned()
                .collect::<Vec<_>>();
            bindings.retain(|binding| {
                binding.trigger_type != LIFECYCLE_TRIGGER_TYPE || binding.id == active_id
            });
            stale
        };
        for binding in stale {
            self.unregister_binding(&binding, deadline).await?;
        }
        Ok(())
    }

    async fn bind_engine_trigger(
        &self,
        trigger_type: &'static str,
        function_id: &'static str,
        config: Value,
        deadline: Deadline,
    ) -> anyhow::Result<String> {
        let response = self
            .client
            .call_with_deadline(
                "engine::register_trigger",
                json!({
                    "trigger_type": trigger_type,
                    "function_id": function_id,
                    "config": config
                }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(anyhow::Error::msg)?;
        let registered: RegisterTriggerResponse =
            serde_json::from_value(response).map_err(|error| {
                anyhow::anyhow!("invalid {trigger_type} registration response: {error}")
            })?;
        let id = registered.id;
        self.bindings
            .lock()
            .map_err(|_| anyhow::anyhow!("integration/bindings_lock_poisoned"))?
            .push(BoundTrigger {
                id: id.clone(),
                trigger_type,
            });
        Ok(id)
    }

    async fn unregister_binding(
        &self,
        binding: &BoundTrigger,
        deadline: Deadline,
    ) -> anyhow::Result<()> {
        self.client
            .call_with_deadline(
                "engine::unregister_trigger",
                json!({
                    "id": binding.id,
                    "trigger_type": binding.trigger_type
                }),
                deadline,
                DEFAULT_CALL_TIMEOUT_MS,
            )
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }

    pub async fn wait_until_ready(
        &self,
        required_functions: &[&str],
        deadline: Deadline,
    ) -> anyhow::Result<()> {
        loop {
            let ready_notified = self.ready_notify.notified();
            let functions_notified = self.functions_notify.notified();
            let ready = self.ready.load(Ordering::Acquire);
            let functions_ready = {
                let functions = self
                    .functions
                    .lock()
                    .map_err(|_| anyhow::anyhow!("integration/functions_lock_poisoned"))?;
                required_functions
                    .iter()
                    .all(|function_id| functions.contains(*function_id))
            };
            if ready && functions_ready {
                return Ok(());
            }

            if let Err(error) = deadline
                .timeout("harness readiness signals", async {
                    tokio::select! {
                        _ = ready_notified => {}
                        _ = functions_notified => {}
                    }
                })
                .await
            {
                let ready = self.ready.load(Ordering::Acquire);
                let missing_functions = {
                    let functions = self
                        .functions
                        .lock()
                        .map_err(|_| anyhow::anyhow!("integration/functions_lock_poisoned"))?;
                    required_functions
                        .iter()
                        .filter(|function_id| !functions.contains(**function_id))
                        .copied()
                        .collect::<Vec<_>>()
                };
                anyhow::bail!("{error}; ready={ready}; missing_functions={missing_functions:?}");
            }
        }
    }

    pub async fn wait_for_completion(
        &self,
        deadline: Deadline,
    ) -> anyhow::Result<CompletionObservation> {
        self.wait_for_completion_turns(1, deadline)
            .await?
            .into_iter()
            .rev()
            .find(|observation| observation.event.terminal)
            .ok_or_else(|| anyhow::anyhow!("completion wait returned no events"))
    }

    pub async fn wait_for_completion_turns(
        &self,
        expected: usize,
        deadline: Deadline,
    ) -> anyhow::Result<Vec<CompletionObservation>> {
        anyhow::ensure!(expected > 0, "expected completion count must be positive");
        loop {
            let notified = self.completion_notify.notified();
            let observations = self
                .completions
                .lock()
                .map_err(|_| anyhow::anyhow!("completion observer lock poisoned"))?
                .clone();
            let turns = observations
                .iter()
                .filter(|observation| observation.event.terminal)
                .map(|observation| observation.event.turn_id.as_str())
                .collect::<BTreeSet<_>>();
            if turns.len() >= expected {
                return Ok(observations);
            }
            deadline
                .timeout("terminal turn completion", notified)
                .await?;
        }
    }

    pub async fn wait_for_trace_change(
        &self,
        after_generation: u64,
        deadline: Deadline,
    ) -> anyhow::Result<u64> {
        loop {
            let notified = self.trace_notify.notified();
            let generation = self.trace_generation.load(Ordering::Acquire);
            if generation > after_generation {
                return Ok(generation);
            }
            deadline.timeout("trace-store change", notified).await?;
        }
    }

    pub async fn shutdown(&self) {
        let bindings = self
            .bindings
            .lock()
            .map(|mut bindings| std::mem::take(&mut *bindings))
            .unwrap_or_default();
        let deadline = Deadline::after(SHUTDOWN_TIMEOUT);
        for binding in bindings.into_iter().rev() {
            if let Err(error) = self
                .client
                .call_with_deadline(
                    "engine::unregister_trigger",
                    json!({
                        "id": binding.id,
                        "trigger_type": binding.trigger_type
                    }),
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await
            {
                tracing::warn!(
                    trigger_type = binding.trigger_type,
                    %error,
                    "integration observer trigger cleanup failed"
                );
            }
        }
        self.client.shutdown().await;
    }
}

fn parse_lifecycle_payload(mut payload: Value) -> Result<LifecycleEventV1, String> {
    strip_engine_fields(&mut payload);
    serde_json::from_value(payload)
        .map_err(|error| format!("integration/lifecycle_contract: {error}"))
}

fn parse_ready_payload(payload: &Value) -> Result<(), String> {
    if payload.get("status").and_then(Value::as_str) == Some("ready") {
        Ok(())
    } else {
        Err("integration/ready_contract: expected status=ready".to_string())
    }
}

fn parse_function_ids(payload: Value) -> Result<BTreeSet<String>, String> {
    let payload: FunctionsChangedPayload = serde_json::from_value(payload)
        .map_err(|error| format!("integration/functions_contract: {error}"))?;
    if payload.event != "functions_changed" {
        return Err(format!(
            "integration/functions_contract: unexpected event {}",
            payload.event
        ));
    }
    Ok(payload
        .functions
        .into_iter()
        .map(|function| function.function_id)
        .collect())
}

fn parse_trace_payload(payload: &Value) -> Result<(), String> {
    let trace_ids = payload
        .get("trace_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "integration/trace_contract: trace_ids must be an array".to_string())?;
    if trace_ids.iter().all(|trace_id| trace_id.as_str().is_some()) {
        Ok(())
    } else {
        Err("integration/trace_contract: trace_ids must contain strings".to_string())
    }
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

    #[test]
    fn observer_payload_parsers_reject_malformed_events() {
        assert!(parse_ready_payload(&json!({ "status": "ready" })).is_ok());
        assert!(parse_ready_payload(&json!({ "status": "booting" })).is_err());

        let functions = parse_function_ids(json!({
            "event": "functions_changed",
            "generation": 7,
            "functions": [
                { "function_id": "harness::send", "worker_name": "harness" },
                { "function_id": "session::messages", "worker_name": "session" }
            ]
        }))
        .unwrap();
        assert!(functions.contains("harness::send"));
        assert!(functions.contains("session::messages"));

        assert!(parse_trace_payload(&json!({ "trace_ids": ["abc"] })).is_ok());
        assert!(parse_trace_payload(&json!({ "trace_ids": [1] })).is_err());
    }
}
