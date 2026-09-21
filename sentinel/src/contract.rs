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
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

/// Where an error event came from. Part of the fingerprint: the same text
/// seen as a span and as a log is two groups, because one has a trace to
/// investigate and the other does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorSourceV1 {
    /// A span with status `error`.
    Trace,
    /// An OTel log record at ERROR.
    Log,
    /// Reserved for the adapters the spec designs but v1 does not implement.
    HarnessTurn,
    Report,
}

impl ErrorSourceV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Log => "log",
            Self::HarnessTurn => "harness-turn",
            Self::Report => "report",
        }
    }
}

/// The lifecycle of a group. `resolved` and `ignored` are human decisions;
/// `regressed` is the one the ingest makes on its own, and the reason the
/// worker exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupStatusV1 {
    #[default]
    New,
    /// The first pass of an investigation is running.
    Investigating,
    Diagnosed,
    Resolved,
    /// A new occurrence after a resolve.
    Regressed,
    Ignored,
}

impl GroupStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Investigating => "investigating",
            Self::Diagnosed => "diagnosed",
            Self::Resolved => "resolved",
            Self::Regressed => "regressed",
            Self::Ignored => "ignored",
        }
    }

    /// Whether the group is one the open list shows.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::New | Self::Investigating | Self::Diagnosed | Self::Regressed
        )
    }
}

/// How long an ignore lasts. Every rule keeps counting occurrences; what
/// changes is when the group comes back to the open list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IgnoreRuleV1 {
    Forever,
    /// Reopens after this many further occurrences.
    Occurrences {
        count: u64,
    },
    /// Reopens when the worker version changes.
    VersionChange,
}

/// What the ignore rule is measured against, captured when it was set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IgnoreBaselineV1 {
    pub occurrence_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_version: Option<String>,
}

/// Why a group changed, for the event siblings subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupChangeReasonV1 {
    Regression,
    IgnoreExpired,
    Resolved,
    Ignored,
    Diagnosed,
    Reopened,
    Investigating,
}

/// A group as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupSummaryV1 {
    pub id: String,
    pub fingerprint: String,
    pub source: ErrorSourceV1,
    pub namespace: String,
    /// The worker that owns the failing function.
    pub service_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception_type: Option<String>,
    pub title: String,
    pub status: GroupStatusV1,
    pub occurrence_count: u64,
    /// Distinct sessions this failure touched. Counted from its own table, so
    /// it stays true after occurrence rows are pruned.
    pub sessions_affected: u64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_version: Option<String>,
    /// Occurrences per hour for the last day, oldest first.
    pub sparkline: Vec<u64>,
    pub has_diagnosis: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore_rule: Option<IgnoreRuleV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regressed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupsListRequestV1 {
    /// Defaults to the open states.
    #[serde(default)]
    pub status: Option<Vec<GroupStatusV1>>,
    #[serde(default)]
    pub service_name: Option<String>,
    /// Only groups seen since this moment.
    #[serde(default)]
    pub since_ms: Option<i64>,
    /// Matches the title, the message sample and the function id.
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupsListResponseV1 {
    pub groups: Vec<GroupSummaryV1>,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupGetRequestV1 {
    pub group_id: String,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

impl GroupGetRequestV1 {
    pub fn new(group_id: impl Into<String>) -> Self {
        Self {
            group_id: group_id.into(),
            _caller_worker_id: None,
        }
    }
}

/// One recorded instance of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceSummaryV1 {
    pub id: String,
    pub at_ms: i64,
    pub source: ErrorSourceV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_version: Option<String>,
    pub message: String,
    /// Whether the frozen bundle is still kept for this one.
    pub has_evidence: bool,
    /// Whether the trace was re-read after the ancestors closed.
    pub settled: bool,
    /// The owner could not be pinned to one namespace.
    pub namespace_ambiguous: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupGetResponseV1 {
    pub group: GroupSummaryV1,
    /// The message exactly as it was captured, after redaction.
    pub message_sample: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_occurrence: Option<OccurrenceSummaryV1>,
    /// Whether the trace behind the latest occurrence is still in the
    /// engine's ring. False means the frozen snapshot is all there is.
    pub trace_available: bool,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OccurrencesListRequestV1 {
    pub group_id: String,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OccurrencesListResponseV1 {
    pub occurrences: Vec<OccurrenceSummaryV1>,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceGetRequestV1 {
    pub occurrence_id: String,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceGetResponseV1 {
    /// Absent when retention has pruned it: the row stays, the bundle goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<crate::evidence::EvidenceBundleV1>,
    pub pruned: bool,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequestV1 {
    pub group_id: String,
    /// Occurrences on the version that was resolved keep counting without
    /// reopening the group — the fix is not deployed here yet.
    #[serde(default)]
    pub until_version_change: bool,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IgnoreRequestV1 {
    pub group_id: String,
    pub rule: IgnoreRuleV1,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupActionRequestV1 {
    pub group_id: String,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

impl GroupActionRequestV1 {
    pub fn new(group_id: impl Into<String>) -> Self {
        Self {
            group_id: group_id.into(),
            _caller_worker_id: None,
        }
    }
}

/// The state a group was left in, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupStateResponseV1 {
    pub group_id: String,
    pub status: GroupStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<GroupChangeReasonV1>,
}

/// The event siblings and the console subscribe to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupChangedEventV1 {
    pub op: GroupChangedOpV1,
    pub group_id: String,
    pub status: GroupStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_status: Option<GroupStatusV1>,
    pub occurrence_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<GroupChangeReasonV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupChangedOpV1 {
    Created,
    /// A new occurrence on a group that is otherwise unchanged. Coalesced.
    Occurrence,
    Status,
}

/// What a subscriber may narrow `sentinel::group-changed` to.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct GroupChangedConfigV1 {
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub ops: Option<Vec<GroupChangedOpV1>>,
    #[serde(default)]
    pub service_name: Option<String>,
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
