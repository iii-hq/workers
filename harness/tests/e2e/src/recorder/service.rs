//! SDK-facing recorder service and controlled-function registration.

use std::path::PathBuf;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

use super::store::EventStore;
use crate::client::Client;
use crate::types::recorder::{
    LifecycleAcceptedV1, LifecycleEventV1, RecorderConfigV1, RecorderEventKind, RecorderEventV1,
    RecorderTargetV1,
};

const LIFECYCLE_FUNCTION_ID: &str = "integration-recorder::lifecycle";

pub struct Recorder {
    client: Client,
    store: Arc<EventStore>,
    event_notify: Arc<tokio::sync::Notify>,
}

impl Recorder {
    pub async fn start(ws_url: &str, log_path: PathBuf) -> anyhow::Result<Self> {
        let client = Client::connect(ws_url, "integration-recorder");
        let store = Arc::new(EventStore::new(log_path));
        let recorder = Recorder {
            client,
            store,
            event_notify: Arc::new(tokio::sync::Notify::new()),
        };
        recorder.register_lifecycle();
        Ok(recorder)
    }

    fn register_lifecycle(&self) {
        let store = self.store.clone();
        let event_notify = Arc::clone(&self.event_notify);
        self.client.inner().register_function(
            LIFECYCLE_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let store = store.clone();
                let event_notify = Arc::clone(&event_notify);
                async move {
                    let payload = parse_lifecycle_payload(payload).map_err(Error::Handler)?;
                    append_handler_event(
                        &store,
                        RecorderEventKind::Lifecycle,
                        LIFECYCLE_FUNCTION_ID,
                        payload,
                        "integration/lifecycle",
                    )?;
                    event_notify.notify_waiters();
                    serde_json::to_value(LifecycleAcceptedV1 { accepted: true }).map_err(|error| {
                        Error::Handler(format!("integration/lifecycle_response_serialize: {error}"))
                    })
                }
            })
            .description("Durable sink for harness lifecycle trigger deliveries.")
            .request_format(schema_for::<LifecycleEventV1>())
            .response_format(schema_for::<LifecycleAcceptedV1>()),
        );
    }

    /// Configure and register the run-scoped controlled functions directly.
    pub fn configure(&self, run_id: &str, config: &RecorderConfigV1) -> anyhow::Result<()> {
        let prefix = format!("{run_id}::");
        anyhow::ensure!(
            config.target.function_id.starts_with(&prefix),
            "integration/target_scope: {} must be prefixed by {prefix}",
            config.target.function_id
        );

        self.store.configure(run_id)?;
        register_controlled_function(
            self.client.inner(),
            &self.store,
            &config.target,
            Arc::clone(&self.event_notify),
        );
        Ok(())
    }

    /// Durably clear evidence for a run and restart event sequencing.
    pub fn reset(&self, run_id: &str) -> anyhow::Result<u64> {
        self.store.reset(run_id)
    }

    /// Return the ordered in-process snapshot.
    pub fn snapshot(&self) -> anyhow::Result<Vec<RecorderEventV1>> {
        self.store.snapshot()
    }

    /// Wait for the lifecycle trigger delivery without repeatedly calling a
    /// harness status function. The notification is armed before inspecting
    /// the store, so an event arriving between those operations cannot be
    /// missed.
    pub async fn wait_for_lifecycle(
        &self,
        deadline: crate::deadline::Deadline,
    ) -> anyhow::Result<RecorderEventV1> {
        self.wait_for_lifecycle_turns(1, deadline)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("lifecycle wait returned no events"))
    }

    /// Wait until lifecycle evidence contains the requested number of
    /// distinct terminal turns. Duplicate at-least-once deliveries do not
    /// advance the count.
    pub async fn wait_for_lifecycle_turns(
        &self,
        expected: usize,
        deadline: crate::deadline::Deadline,
    ) -> anyhow::Result<Vec<RecorderEventV1>> {
        anyhow::ensure!(
            expected > 0,
            "expected lifecycle turn count must be positive"
        );
        loop {
            let notified = self.event_notify.notified();
            let events: Vec<_> = self
                .snapshot()?
                .into_iter()
                .filter(|event| event.kind == RecorderEventKind::Lifecycle)
                .collect();
            let turns = events
                .iter()
                .filter_map(|event| event.payload.get("turn_id").and_then(Value::as_str))
                .collect::<std::collections::BTreeSet<_>>();
            if turns.len() >= expected {
                return Ok(events);
            }
            deadline
                .timeout("lifecycle terminal turn deliveries", notified)
                .await?;
        }
    }

    /// Create the lifecycle trigger binding (runner's Arm step). The binding
    /// filter uses the pre-chosen session id so parallel-unrelated sessions
    /// (there are none in v1, but the filter is part of the contract) never
    /// deliver here.
    pub async fn bind_lifecycle(&self, trigger_type: &str, session_id: &str) -> anyhow::Result<()> {
        self.client
            .inner()
            .register_trigger(RegisterTriggerInput {
                trigger_type: trigger_type.to_string(),
                function_id: LIFECYCLE_FUNCTION_ID.to_string(),
                config: json!({ "session_id": session_id }),
                metadata: None,
            })
            .map_err(|error| {
                anyhow::anyhow!("binding {trigger_type} -> {LIFECYCLE_FUNCTION_ID} failed: {error}")
            })?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

pub(super) fn parse_lifecycle_payload(payload: Value) -> Result<Value, String> {
    let payload = strip_engine_fields(payload);
    let event: LifecycleEventV1 = serde_json::from_value(payload)
        .map_err(|error| format!("integration/lifecycle_contract: {error}"))?;
    serde_json::to_value(event).map_err(|error| format!("integration/lifecycle_serialize: {error}"))
}

/// Register the declared controlled function with durable recording.
fn register_controlled_function(
    iii: &iii_sdk::IIIClient,
    store: &Arc<EventStore>,
    declared: &RecorderTargetV1,
    event_notify: Arc<tokio::sync::Notify>,
) {
    let response = declared.response.clone();
    let store = store.clone();
    let function_id = declared.function_id.clone();
    let handler_function_id = function_id.clone();
    iii.register_function(
        &function_id,
        RegisterFunction::new_async(move |payload: Value| {
            let store = store.clone();
            let event_notify = Arc::clone(&event_notify);
            let response = response.clone();
            let function_id = handler_function_id.clone();
            async move {
                append_handler_event(
                    &store,
                    RecorderEventKind::TargetCall,
                    &function_id,
                    payload,
                    "integration/target_append",
                )?;
                event_notify.notify_waiters();
                Ok::<Value, Error>(response)
            }
        })
        .description(declared.description.clone())
        .request_format(Value::Object(declared.request_schema.clone()))
        .response_format(const_response_schema(&declared.response)),
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

fn append_handler_event(
    store: &EventStore,
    kind: RecorderEventKind,
    function_id: &str,
    payload: Value,
    error_context: &str,
) -> Result<(), Error> {
    store
        .append(kind, function_id, strip_engine_fields(payload))
        .map(|_| ())
        .map_err(|error| Error::Handler(format!("{error_context}: {error:#}")))
}

/// Remove engine-injected `_`-prefixed members from a payload's top level
/// before a strict (`deny_unknown_fields`) parse.
pub(super) fn strip_engine_fields(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.retain(|key, _| !key.starts_with('_'));
    }
    value
}
