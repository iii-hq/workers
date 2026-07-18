//! The controlled recorder: run-scoped target functions and the lifecycle
//! sink are registered with the engine, while configuration, reset, and
//! snapshots remain direct runner operations. Every accepted target or
//! lifecycle call is durably appended to `<run>/recorder.log.jsonl`
//! (write + fsync) before the handler responds.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

use crate::client::Client;
use crate::types::recorder::{RecorderConfigV1, RecorderEventKind, RecorderEventV1};
use crate::types::script::SchemaVersion1;

struct EventStoreState {
    run_id: Option<String>,
    next_sequence: u64,
    events: Vec<RecorderEventV1>,
}

struct EventStore {
    state: Mutex<EventStoreState>,
    log_path: PathBuf,
}

impl EventStore {
    fn new(log_path: PathBuf) -> Self {
        Self {
            state: Mutex::new(EventStoreState {
                run_id: None,
                next_sequence: 1,
                events: Vec::new(),
            }),
            log_path,
        }
    }

    fn configure(&self, run_id: &str) -> anyhow::Result<()> {
        let mut state = self.lock()?;
        state.run_id = Some(run_id.to_string());
        Ok(())
    }

    /// Durably truncate the event log before resetting the in-memory view.
    fn reset(&self, run_id: &str) -> anyhow::Result<u64> {
        let mut state = self.lock()?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.log_path)
            .with_context(|| {
                format!("open recorder log for reset at {}", self.log_path.display())
            })?;
        file.sync_all()
            .with_context(|| format!("fsync recorder log reset at {}", self.log_path.display()))?;

        state.run_id = Some(run_id.to_string());
        state.events.clear();
        state.next_sequence = 1;
        Ok(state.next_sequence)
    }

    /// Durably append one event before making it visible to callers.
    fn append(
        &self,
        kind: RecorderEventKind,
        function_id: &str,
        payload: Value,
    ) -> anyhow::Result<u64> {
        let mut state = self.lock()?;
        let sequence = state.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .context("recorder event sequence exhausted")?;
        let run_id = state
            .run_id
            .clone()
            .context("recorder event store is not configured")?;
        let event = RecorderEventV1 {
            schema_version: SchemaVersion1::V1,
            run_id,
            sequence,
            kind,
            function_id: function_id.to_string(),
            payload,
            received_at: now_rfc3339(),
        };

        let mut line = serde_json::to_vec(&event).context("serialize recorder event")?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .with_context(|| {
                format!(
                    "open recorder log for append at {}",
                    self.log_path.display()
                )
            })?;
        file.write_all(&line)
            .with_context(|| format!("write recorder log at {}", self.log_path.display()))?;
        file.sync_all()
            .with_context(|| format!("fsync recorder log at {}", self.log_path.display()))?;

        state.next_sequence = next_sequence;
        state.events.push(event);
        Ok(sequence)
    }

    fn snapshot(&self, after_sequence: Option<u64>) -> anyhow::Result<Vec<RecorderEventV1>> {
        let after_sequence = after_sequence.unwrap_or(0);
        let state = self.lock()?;
        Ok(state
            .events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect())
    }

    fn lock(&self) -> anyhow::Result<std::sync::MutexGuard<'_, EventStoreState>> {
        self.state
            .lock()
            .map_err(|_| anyhow::anyhow!("recorder event store lock poisoned"))
    }
}

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
        let iii = self.client.inner();

        // integration-recorder::lifecycle — the trigger-bound sink. Payload
        // is the exact harness lifecycle event.
        {
            let store = self.store.clone();
            iii.register_function(
                "integration-recorder::lifecycle",
                RegisterFunction::new_async(move |payload: Value| {
                    let store = store.clone();
                    async move {
                        store
                            .append(
                                RecorderEventKind::Lifecycle,
                                "integration-recorder::lifecycle",
                                strip_engine_fields(payload),
                            )
                            .map_err(|error| {
                                Error::Handler(format!("integration/lifecycle: {error:#}"))
                            })?;
                        Ok::<Value, Error>(json!({ "accepted": true }))
                    }
                })
                .description("Durable sink for harness lifecycle trigger deliveries."),
            );
        }
    }

    /// Configure and register the run-scoped controlled functions directly.
    /// Returns the canonical digest of the target request schema.
    pub fn configure(&self, run_id: &str, config: &RecorderConfigV1) -> anyhow::Result<String> {
        let prefix = format!("{run_id}::");
        let mut function_ids = BTreeSet::new();
        for declared in std::iter::once(&config.target).chain(config.extra_functions.iter()) {
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
        for declared in std::iter::once(&config.target).chain(config.extra_functions.iter()) {
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
            "integration-recorder::lifecycle",
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

    pub async fn shutdown(&self) {
        self.client.shutdown().await;
    }
}

/// Register one declared controlled function: verbatim description/schema,
/// durable append per call, declared response (after the optional
/// fault-injection delay).
fn register_controlled_function(
    iii: &iii_sdk::IIIClient,
    store: &Arc<EventStore>,
    declared: &crate::types::recorder::RecorderTargetV1,
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
                store
                    .append(
                        RecorderEventKind::TargetCall,
                        &function_id,
                        strip_engine_fields(payload),
                    )
                    .map_err(|error| {
                        Error::Handler(format!("integration/target_append: {error:#}"))
                    })?;
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::{EventStore, RecorderEventKind};

    #[test]
    fn event_store_persists_ordered_events_and_resets_durably() {
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("recorder.log.jsonl");
        let store = EventStore::new(log_path.clone());

        assert_eq!(store.reset("run-1").expect("reset"), 1);
        assert_eq!(
            store
                .append(
                    RecorderEventKind::TargetCall,
                    "run-1::target",
                    json!({"x": 1})
                )
                .expect("first append"),
            1
        );
        assert_eq!(
            store
                .append(
                    RecorderEventKind::Lifecycle,
                    "integration-recorder::lifecycle",
                    json!({"session_id": "session-1"}),
                )
                .expect("second append"),
            2
        );

        let snapshot = store.snapshot(None).expect("snapshot");
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].sequence, 1);
        assert_eq!(snapshot[1].sequence, 2);
        assert_eq!(
            store.snapshot(Some(1)).expect("filtered snapshot"),
            vec![snapshot[1].clone()]
        );

        let persisted: Vec<serde_json::Value> = std::fs::read_to_string(&log_path)
            .expect("read log")
            .lines()
            .map(|line| serde_json::from_str(line).expect("event JSON"))
            .collect();
        assert_eq!(
            persisted,
            snapshot
                .iter()
                .map(|event| serde_json::to_value(event).expect("event value"))
                .collect::<Vec<_>>()
        );

        assert_eq!(store.reset("run-2").expect("second reset"), 1);
        assert!(store.snapshot(None).expect("empty snapshot").is_empty());
        assert_eq!(std::fs::read(&log_path).expect("read reset log"), b"");
        assert_eq!(
            store
                .append(RecorderEventKind::TargetCall, "run-2::target", json!({}))
                .expect("append after reset"),
            1
        );
    }

    #[test]
    fn failed_open_is_returned_without_acknowledging_the_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let parent = temp.path().join("missing");
        let store = EventStore::new(parent.join("recorder.log.jsonl"));
        store.configure("run-1").expect("configure");

        let error = store
            .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
            .expect_err("open must fail");
        assert!(
            format!("{error:#}").contains("open recorder log for append"),
            "{error:#}"
        );
        assert!(store.snapshot(None).expect("snapshot").is_empty());

        std::fs::create_dir(&parent).expect("create log parent");
        assert_eq!(
            store
                .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
                .expect("retry append"),
            1,
            "a failed append must not advance the acknowledged sequence"
        );
    }

    #[test]
    fn events_are_rejected_until_the_store_is_configured() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = EventStore::new(temp.path().join("recorder.log.jsonl"));

        let error = store
            .append(RecorderEventKind::Lifecycle, "lifecycle", json!({}))
            .expect_err("unconfigured append must fail");
        assert!(
            format!("{error:#}").contains("event store is not configured"),
            "{error:#}"
        );
        assert!(store.snapshot(None).expect("snapshot").is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn write_errors_are_returned_without_acknowledging_the_event() {
        let store = EventStore::new(Path::new("/dev/full").to_path_buf());
        store.configure("run-1").expect("configure");

        let error = store
            .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
            .expect_err("/dev/full must reject the write");
        assert!(
            format!("{error:#}").contains("write recorder log"),
            "{error:#}"
        );
        assert!(store.snapshot(None).expect("snapshot").is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fsync_errors_are_returned_without_acknowledging_the_event() {
        let store = EventStore::new(Path::new("/dev/null").to_path_buf());
        store.configure("run-1").expect("configure");

        let error = store
            .append(RecorderEventKind::TargetCall, "run-1::target", json!({}))
            .expect_err("/dev/null cannot be fsynced");
        assert!(
            format!("{error:#}").contains("fsync recorder log"),
            "{error:#}"
        );
        assert!(store.snapshot(None).expect("snapshot").is_empty());
    }
}
