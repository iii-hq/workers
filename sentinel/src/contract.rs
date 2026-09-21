//! Wire types.
//!
//! Requests tolerate the `_caller_worker_id` the engine injects while still
//! rejecting every other unknown field: a typo in an agent's payload is an
//! error the agent can read and fix, not a silently dropped argument.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusRequestV1 {
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema.
    #[serde(default)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

/// State of the engine's in-memory telemetry stores, as the worker last
/// observed them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceStoreStateV1 {
    /// Spans are being served.
    Memory,
    /// `engine::traces::*` reports its memory exporter is off: the worker
    /// runs on logs alone and the console shows the same empty state the
    /// traces page does.
    Disabled,
    /// Not observed yet.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EngineStatusV1 {
    pub trace_store: TraceStoreStateV1,
    pub logs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourcesStatusV1 {
    pub trace: bool,
    pub log: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestStatusV1 {
    /// Jobs handed to the durable queue and not yet completed.
    pub queued: u64,
    pub processed_total: u64,
    pub deduped: u64,
    /// Traces whose root is this worker — the cut that stops the monitor from
    /// monitoring itself.
    pub dropped_own_trace: u64,
    /// Traces tagged with an investigation session.
    pub dropped_investigation: u64,
    pub dropped_ignored_service: u64,
    /// Ticks for the ingest's own traces, filtered before the queue.
    pub phantom_ticks_dropped: u64,
    /// ERROR logs currently waiting for the error span of their trace.
    pub logs_pending_join: u64,
    /// Logs promoted without resolving their trace: a bounded, visible gap.
    pub logs_unattributed: u64,
    /// Values rewritten as `[redacted:<kind>]` on capture, all sources.
    pub redactions: u64,
    /// Traces that had already left the engine's ring when the job ran.
    pub lost_before_capture: u64,
    /// Deliveries refused because the durable dependencies were not claimed
    /// yet. They are redelivered, not lost.
    pub dropped_not_ready: u64,
    /// Jobs that exhausted their retries.
    pub dropped_failed: u64,
    /// Set while the ingest breaker is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupCountsV1 {
    pub open: u64,
    pub regressed: u64,
    pub ignored: u64,
    pub resolved: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationCountsV1 {
    /// First passes running right now.
    pub running: u64,
    /// Investigation sessions that are still open for conversation.
    pub open_sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryStatusV1 {
    pub id: String,
    pub path: String,
    /// Whether the checkout is on this machine right now. False disables code
    /// access for its workers; grouping and evidence are unaffected.
    pub exists: bool,
    pub workers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusResponseV1 {
    /// True once the configuration says so *and* the durable dependencies are
    /// claimed. False means nothing is being ingested.
    pub enabled: bool,
    pub engine: EngineStatusV1,
    pub sources: SourcesStatusV1,
    pub ingest: IngestStatusV1,
    pub groups: GroupCountsV1,
    pub investigations: InvestigationCountsV1,
    pub repositories: Vec<RepositoryStatusV1>,
    /// Why the stored configuration was refused, when it was. The worker runs
    /// on shipped defaults and stays disabled until a valid value arrives, so
    /// a typo reads as a named field here instead of a silent idle worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_accepts_engine_metadata_but_not_a_typo() {
        serde_json::from_value::<StatusRequestV1>(serde_json::json!({
            "_caller_worker_id": "console"
        }))
        .expect("engine metadata is accepted");

        serde_json::from_value::<StatusRequestV1>(serde_json::json!({ "verbose": true }))
            .expect_err("an unknown field is rejected");
    }

    #[test]
    fn the_caller_id_stays_out_of_the_published_schema() {
        let schema = serde_json::to_value(schemars::schema_for!(StatusRequestV1))
            .expect("schema serializes");
        assert!(
            !schema.to_string().contains("_caller_worker_id"),
            "engine metadata must not be advertised as a request field: {schema}"
        );
        assert_eq!(schema["type"], "object");
    }
}
