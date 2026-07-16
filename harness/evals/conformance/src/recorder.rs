//! The controlled recorder (spec § Proposed recorder contract): five fixed
//! control functions, one run-scoped target function, and the lifecycle
//! sink. Every accepted target/lifecycle call is durably appended to
//! `<run>/recorder.log.jsonl` (write + fsync) *before* the handler responds,
//! with a strictly increasing sequence. `snapshot` orders by sequence;
//! `await` is a deadline-bounded convenience over the same log.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

use crate::client::Client;
use crate::types::recorder::{
    RecorderConfigV1, RecorderConfigureRequestV1, RecorderEventKind, RecorderEventV1,
};
use crate::types::script::SchemaVersion1;

struct State {
    run_id: Option<String>,
    next_sequence: u64,
    events: Vec<RecorderEventV1>,
    log_path: PathBuf,
    /// The target registered by `configure` (used to enforce single
    /// configuration per run and by the runner's digest verification).
    configured_target: Option<RecorderConfigV1>,
}

impl State {
    /// Durably append one event before the caller's handler responds.
    fn append(&mut self, kind: RecorderEventKind, function_id: &str, payload: Value) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        let event = RecorderEventV1 {
            schema_version: SchemaVersion1::V1,
            run_id: self.run_id.clone().unwrap_or_default(),
            sequence,
            kind,
            function_id: function_id.to_string(),
            payload,
            received_at: now_rfc3339(),
        };
        let line = serde_json::to_string(&event).expect("recorder event serializes");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(file, "{line}");
            let _ = file.sync_all();
        }
        self.events.push(event);
        sequence
    }
}

pub struct Recorder {
    client: Client,
    state: Arc<Mutex<State>>,
}

impl Recorder {
    pub async fn start(ws_url: &str, log_path: PathBuf) -> anyhow::Result<Self> {
        let client = Client::connect(ws_url, "conformance-recorder");
        let state = Arc::new(Mutex::new(State {
            run_id: None,
            next_sequence: 1,
            events: Vec::new(),
            log_path,
            configured_target: None,
        }));
        let recorder = Recorder { client, state };
        recorder.register_controls();
        Ok(recorder)
    }

    fn register_controls(&self) {
        let iii = self.client.inner();

        // conformance-recorder::configure — registers the run-scoped target
        // verbatim and returns the canonical schema digest.
        {
            let state = self.state.clone();
            let iii_for_target = iii.clone();
            iii.register_function(
                "conformance-recorder::configure",
                RegisterFunction::new_async(move |raw: Value| {
                    let state = state.clone();
                    let iii = iii_for_target.clone();
                    async move {
                        // The engine stamps internal `_`-prefixed fields
                        // (e.g. `_caller_worker_id`) onto trigger payloads;
                        // strip them before the deny-unknown-fields parse.
                        let request: RecorderConfigureRequestV1 =
                            serde_json::from_value(strip_engine_fields(raw)).map_err(|e| {
                                Error::Handler(format!("conformance/configure: {e}"))
                            })?;
                        let target = &request.config.target;
                        let prefix = format!("{}::", request.run_id);
                        let schema = Value::Object(target.request_schema.clone());
                        let digest = crate::canonical::sha256_of_canonical(&schema);

                        {
                            let mut state = state.lock().expect("recorder state");
                            state.run_id = Some(request.run_id.clone());
                            state.configured_target = Some(request.config.clone());
                        }

                        // Register the declared surfaces verbatim (the target
                        // plus any extra controlled functions, e.g. hooks);
                        // each handler answers with its declared response
                        // after a durable append.
                        for declared in
                            std::iter::once(target).chain(request.config.extra_functions.iter())
                        {
                            if !declared.function_id.starts_with(&prefix) {
                                return Err(Error::Handler(format!(
                                    "conformance/target_scope: {} must be prefixed by {prefix}",
                                    declared.function_id
                                )));
                            }
                            register_controlled_function(&iii, &state, declared);
                        }

                        Ok(json!({ "schema_version": "1", "target_schema_sha256": digest }))
                    }
                })
                .description("Configure the run-scoped conformance target and lifecycle binding."),
            );
        }

        // conformance-recorder::reset — clears only the current run; idempotent.
        {
            let state = self.state.clone();
            iii.register_function(
                "conformance-recorder::reset",
                RegisterFunction::new_async(move |request: Value| {
                    let state = state.clone();
                    async move {
                        let run_id = request
                            .get("run_id")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                Error::Handler("conformance/reset: run_id required".into())
                            })?
                            .to_string();
                        let mut state = state.lock().expect("recorder state");
                        state.run_id = Some(run_id);
                        state.events.clear();
                        state.next_sequence = 1;
                        let _ = std::fs::write(&state.log_path, b"");
                        Ok::<Value, Error>(json!({ "schema_version": "1", "next_sequence": 1 }))
                    }
                })
                .description("Reset the recorder's durable log for the current run."),
            );
        }

        // conformance-recorder::snapshot — ordered by sequence.
        {
            let state = self.state.clone();
            iii.register_function(
                "conformance-recorder::snapshot",
                RegisterFunction::new_async(move |request: Value| {
                    let state = state.clone();
                    async move {
                        let after = request
                            .get("after_sequence")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        let state = state.lock().expect("recorder state");
                        let events: Vec<&RecorderEventV1> =
                            state.events.iter().filter(|e| e.sequence > after).collect();
                        Ok::<Value, Error>(json!({ "schema_version": "1", "events": events }))
                    }
                })
                .description("Read the recorder's durable event log, ordered by sequence."),
            );
        }

        // conformance-recorder::await — deadline-bounded count watch.
        {
            let state = self.state.clone();
            iii.register_function(
                "conformance-recorder::await",
                RegisterFunction::new_async(move |request: Value| {
                    let state = state.clone();
                    async move {
                        let kind: RecorderEventKind = serde_json::from_value(
                            request.get("kind").cloned().unwrap_or(Value::Null),
                        )
                        .map_err(|e| Error::Handler(format!("conformance/await: {e}")))?;
                        let count = request.get("count").and_then(Value::as_u64).unwrap_or(1);
                        let timeout_ms = request
                            .get("timeout_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(10_000);
                        let deadline = tokio::time::Instant::now()
                            + std::time::Duration::from_millis(timeout_ms);
                        loop {
                            let observed = {
                                let state = state.lock().expect("recorder state");
                                state.events.iter().filter(|e| e.kind == kind).count() as u64
                            };
                            if observed >= count || tokio::time::Instant::now() >= deadline {
                                return Ok::<Value, Error>(
                                    json!({ "schema_version": "1", "observed": observed }),
                                );
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                })
                .description("Wait until the durable log holds `count` events of `kind`."),
            );
        }

        // conformance-recorder::lifecycle — the trigger-bound sink. Payload
        // is the exact harness lifecycle event.
        {
            let state = self.state.clone();
            iii.register_function(
                "conformance-recorder::lifecycle",
                RegisterFunction::new_async(move |payload: Value| {
                    let state = state.clone();
                    async move {
                        state.lock().expect("recorder state").append(
                            RecorderEventKind::Lifecycle,
                            "conformance-recorder::lifecycle",
                            strip_engine_fields(payload),
                        );
                        Ok::<Value, Error>(json!({ "accepted": true }))
                    }
                })
                .description("Durable sink for harness lifecycle trigger deliveries."),
            );
        }
    }

    /// Create the lifecycle trigger binding (runner's Arm step). The binding
    /// filter uses the pre-chosen session id so parallel-unrelated sessions
    /// (there are none in v1, but the filter is part of the contract) never
    /// deliver here.
    pub async fn bind_lifecycle(&self, trigger_type: &str, session_id: &str) -> anyhow::Result<()> {
        self.bind(
            trigger_type,
            "conformance-recorder::lifecycle",
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
            .map_err(|e| anyhow::anyhow!("binding {trigger_type} -> {function_id} failed: {e}"))?;
        Ok(())
    }

    /// Direct (in-process) event snapshot for evidence collection.
    pub fn events(&self) -> Vec<RecorderEventV1> {
        self.state.lock().expect("recorder state").events.clone()
    }

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

/// Register one declared controlled function: verbatim description/schema,
/// durable append per call, declared response (after the optional
/// fault-injection delay).
fn register_controlled_function(
    iii: &iii_sdk::IIIClient,
    state: &Arc<Mutex<State>>,
    declared: &crate::types::recorder::RecorderTargetV1,
) {
    let response = declared.response.clone();
    let delay_ms = declared.response_delay_ms.unwrap_or(0);
    let state = state.clone();
    let function_id = declared.function_id.clone();
    let handler_function_id = function_id.clone();
    iii.register_function(
        &function_id,
        RegisterFunction::new_async(move |payload: Value| {
            let state = state.clone();
            let response = response.clone();
            let function_id = handler_function_id.clone();
            async move {
                state.lock().expect("recorder state").append(
                    RecorderEventKind::TargetCall,
                    &function_id,
                    strip_engine_fields(payload),
                );
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

/// Remove engine-injected `_`-prefixed members from a payload's top level
/// before a strict (`deny_unknown_fields`) parse.
fn strip_engine_fields(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.retain(|k, _| !k.starts_with('_'));
    }
    value
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}
