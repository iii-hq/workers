//! Ordered in-memory evidence backed by a durable JSONL log.
//!
//! An event becomes visible and advances the sequence only after its complete
//! JSON line has been written and fsynced.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::Context;
use serde_json::Value;

use crate::types::recorder::{RecorderEventKind, RecorderEventV1};
use crate::types::script::SchemaVersion1;

struct EventStoreState {
    run_id: Option<String>,
    next_sequence: u64,
    events: Vec<RecorderEventV1>,
}

pub(super) struct EventStore {
    state: Mutex<EventStoreState>,
    log_path: PathBuf,
}

impl EventStore {
    pub(super) fn new(log_path: PathBuf) -> Self {
        Self {
            state: Mutex::new(EventStoreState {
                run_id: None,
                next_sequence: 1,
                events: Vec::new(),
            }),
            log_path,
        }
    }

    pub(super) fn configure(&self, run_id: &str) -> anyhow::Result<()> {
        let mut state = self.lock()?;
        state.run_id = Some(run_id.to_string());
        Ok(())
    }

    /// Durably truncate the event log before resetting the in-memory view.
    pub(super) fn reset(&self, run_id: &str) -> anyhow::Result<u64> {
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
    pub(super) fn append(
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

    pub(super) fn snapshot(
        &self,
        after_sequence: Option<u64>,
    ) -> anyhow::Result<Vec<RecorderEventV1>> {
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

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}
