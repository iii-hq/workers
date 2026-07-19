//! SDK-facing recorder service and controlled-function registration.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

use super::store::EventStore;
use crate::client::Client;
use crate::types::recorder::{
    RecorderConfigV1, RecorderEventKind, RecorderEventV1, RecorderTargetV1,
};

const LIFECYCLE_FUNCTION_ID: &str = "integration-recorder::lifecycle";

pub struct Recorder {
    client: Client,
    store: Arc<EventStore>,
}

impl Recorder {
    pub async fn start(ws_url: &str, log_path: PathBuf) -> anyhow::Result<Self> {
        let client = Client::connect(ws_url, "integration-recorder");
        let store = Arc::new(EventStore::new(log_path));
        let recorder = Recorder { client, store };
        recorder.register_lifecycle();
        Ok(recorder)
    }

    fn register_lifecycle(&self) {
        let store = self.store.clone();
        self.client.inner().register_function(
            LIFECYCLE_FUNCTION_ID,
            RegisterFunction::new_async(move |payload: Value| {
                let store = store.clone();
                async move {
                    append_handler_event(
                        &store,
                        RecorderEventKind::Lifecycle,
                        LIFECYCLE_FUNCTION_ID,
                        payload,
                        "integration/lifecycle",
                    )?;
                    Ok::<Value, Error>(json!({ "accepted": true }))
                }
            })
            .description("Durable sink for harness lifecycle trigger deliveries."),
        );
    }

    /// Configure and register the run-scoped controlled functions directly.
    /// Returns the canonical digest of the target request schema.
    pub fn configure(&self, run_id: &str, config: &RecorderConfigV1) -> anyhow::Result<String> {
        let prefix = format!("{run_id}::");
        let mut function_ids = BTreeSet::new();
        for declared in controlled_functions(config) {
            anyhow::ensure!(
                declared.function_id.starts_with(&prefix),
                "integration/target_scope: {} must be prefixed by {prefix}",
                declared.function_id
            );
            anyhow::ensure!(
                function_ids.insert(declared.function_id.as_str()),
                "integration/target_duplicate: {} is declared more than once",
                declared.function_id
            );
        }

        self.store.configure(run_id)?;
        for declared in controlled_functions(config) {
            register_controlled_function(self.client.inner(), &self.store, declared);
        }

        let schema = Value::Object(config.target.request_schema.clone());
        Ok(crate::canonical::sha256_of_canonical(&schema))
    }

    /// Durably clear evidence for a run and restart event sequencing.
    pub fn reset(&self, run_id: &str) -> anyhow::Result<u64> {
        self.store.reset(run_id)
    }

    /// Return an ordered in-process snapshot after the optional sequence.
    pub fn snapshot(&self, after_sequence: Option<u64>) -> anyhow::Result<Vec<RecorderEventV1>> {
        self.store.snapshot(after_sequence)
    }

    /// Create the lifecycle trigger binding (runner's Arm step). The binding
    /// filter uses the pre-chosen session id so parallel-unrelated sessions
    /// (there are none in v1, but the filter is part of the contract) never
    /// deliver here.
    pub async fn bind_lifecycle(&self, trigger_type: &str, session_id: &str) -> anyhow::Result<()> {
        self.bind(
            trigger_type,
            LIFECYCLE_FUNCTION_ID,
            json!({ "session_id": session_id }),
        )
        .await
    }

    /// Create an arbitrary trigger binding on the recorder's connection
    /// (scenario `bindings`, e.g. `harness::hook::pre-trigger` chains).
    pub async fn bind(
        &self,
        trigger_type: &str,
        function_id: &str,
        config: Value,
    ) -> anyhow::Result<()> {
        self.client
            .inner()
            .register_trigger(RegisterTriggerInput {
                trigger_type: trigger_type.to_string(),
                function_id: function_id.to_string(),
                config,
                metadata: None,
            })
            .map_err(|error| {
                anyhow::anyhow!("binding {trigger_type} -> {function_id} failed: {error}")
            })?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

fn controlled_functions(config: &RecorderConfigV1) -> impl Iterator<Item = &RecorderTargetV1> {
    std::iter::once(&config.target).chain(config.extra_functions.iter())
}

/// Register one declared controlled function: verbatim description/schema,
/// durable append per call, declared response (after the optional
/// fault-injection delay).
fn register_controlled_function(
    iii: &iii_sdk::IIIClient,
    store: &Arc<EventStore>,
    declared: &RecorderTargetV1,
) {
    let response = declared.response.clone();
    let delay_ms = declared.response_delay_ms.unwrap_or(0);
    let store = store.clone();
    let function_id = declared.function_id.clone();
    let handler_function_id = function_id.clone();
    iii.register_function(
        &function_id,
        RegisterFunction::new_async(move |payload: Value| {
            let store = store.clone();
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
                if delay_ms > 0 {
                    // Fault-injection window: the call is durably observed
                    // but still executing.
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                }
                Ok::<Value, Error>(response)
            }
        })
        .description(declared.description.clone())
        .request_format(Value::Object(declared.request_schema.clone())),
    );
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
