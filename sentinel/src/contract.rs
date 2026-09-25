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
    /// When the group was last resolved and on which version — what a
    /// regression is measured against, so the page can say what came back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_version: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolve_until_version_change: bool,
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
    /// The diagnosis in force, when an investigation recorded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<DiagnosisRecordV1>,
    /// The first pass running right now, if one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_investigation: Option<InvestigationSummaryV1>,
    /// The most recent investigation of any state — what "Continue in chat"
    /// reopens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_investigation: Option<InvestigationSummaryV1>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
    /// The newest occurrence across every group: when ingest last recorded
    /// something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<i64>,
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

// ── investigation ────────────────────────────────────────────────────────

/// How an investigation was opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationModeV1 {
    /// A first pass runs automatically, then the conversation continues.
    #[default]
    Assisted,
    /// The session is created with the evidence and nothing runs until a
    /// person speaks.
    Chat,
}

impl InvestigationModeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assisted => "assisted",
            Self::Chat => "chat",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationStatusV1 {
    /// The first pass is in flight.
    #[default]
    Running,
    /// The first pass ended.
    Completed,
    Failed,
    Cancelled,
    /// A chat-mode session: no first pass ever ran, the conversation is the
    /// investigation.
    Open,
}

impl InvestigationStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Open => "open",
        }
    }

    /// Parse a stored value, defaulting to `running` — the state a row is
    /// in before anything else was decided about it.
    pub fn parse(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "open" => Self::Open,
            _ => Self::Running,
        }
    }

    /// Whether the harness still owes this investigation an answer.
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationSummaryV1 {
    pub id: String,
    pub group_id: String,
    pub occurrence_id: String,
    /// The harness session. The console opens this beside the page.
    pub session_id: String,
    pub mode: InvestigationModeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_pass_turn_id: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    /// The commit the mapped checkout was on when the pass started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_ref: Option<String>,
    /// The worker version the investigated occurrence ran on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub investigated_version: Option<String>,
    pub status: InvestigationStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Observed after the fact, and informative only: this worker imposes no
    /// ceiling on an investigation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub created_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<i64>,
}

// ── the diagnosis an agent records ───────────────────────────────────────
//
// Every struct below denies unknown fields on purpose. The harness does not
// validate an `agent_trigger` payload against the function's schema — it only
// checks the policy globs — so *this* is the validation: a model that invents
// a field gets a readable error back and corrects itself.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisCategoryV1 {
    Bug,
    Configuration,
    Dependency,
    /// Real, but it went away on its own: a timeout, a restart, a race.
    Transient,
    /// The failure is the designed behaviour of the code.
    Expected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceV1 {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKindV1 {
    /// A place in the repository, as a path relative to its root.
    Code,
    Trace,
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RiskV1 {
    Low,
    Medium,
    High,
}

/// One thing the agent is pointing at, and why it matters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisEvidenceV1 {
    pub kind: EvidenceKindV1,
    /// Relative to the repository root, for `code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The span this points at, for `trace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    /// At most 400 characters of the thing itself.
    pub excerpt: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RootCauseV1 {
    pub description: String,
    pub evidence: Vec<DiagnosisEvidenceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposedFixV1 {
    pub description: String,
    pub files: Vec<String>,
    pub risk: RiskV1,
    pub steps: Vec<String>,
}

/// What an investigation concluded. Written by the agent, never by this
/// worker, and kept verbatim: a later version supersedes it without erasing
/// it, so the same failure diagnosed twice can be compared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisV1 {
    /// One to three sentences, written for a person.
    pub summary: String,
    pub category: DiagnosisCategoryV1,
    pub confidence: ConfidenceV1,
    pub root_cause: RootCauseV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_fix: Option<ProposedFixV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduction: Option<String>,
    /// What would have raised the confidence. An honest `low` with this
    /// filled in is a better answer than a guess.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_evidence: Vec<String>,
    /// Drift between the checkout that was read and the version that failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_note: Option<String>,
    /// Groups the agent believes share this cause. A suggestion the console
    /// shows; nothing is merged on the strength of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_groups: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisSourceV1 {
    /// Recorded while the automatic first pass was running.
    FirstPass,
    /// Recorded later, in the conversation.
    Conversation,
}

impl DiagnosisSourceV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FirstPass => "first_pass",
            Self::Conversation => "conversation",
        }
    }
}

/// A recorded diagnosis with its provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisRecordV1 {
    pub id: String,
    pub investigation_id: String,
    pub group_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub source: DiagnosisSourceV1,
    pub model: String,
    pub created_ms: i64,
    /// False when the stored payload no longer parses as a `DiagnosisV1` —
    /// the raw text is kept so nothing is lost silently.
    pub valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<DiagnosisV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_result: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigateRequestV1 {
    pub group_id: String,
    /// Defaults to `assisted`.
    #[serde(default)]
    pub mode: Option<InvestigationModeV1>,
    /// Catalog id, or a raw model id alongside `provider`. Falls back to the
    /// configured one.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// Defaults to the most recent occurrence that still has evidence.
    #[serde(default)]
    pub occurrence_id: Option<String>,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigateResponseV1 {
    pub investigation_id: String,
    /// What the console opens beside the page.
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_pass_turn_id: Option<String>,
    /// A first pass was already running: this is that one, not a second.
    pub existing: bool,
}

/// The agent's one write. There is no `investigation_id` and no `session_id`
/// here on purpose: identity comes from the invocation's baggage, which the
/// agent does not control, so no payload can point a diagnosis at somebody
/// else's investigation.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisRecordRequestV1 {
    pub group_id: String,
    pub diagnosis: DiagnosisV1,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisRecordResponseV1 {
    pub diagnosis_id: String,
    /// 1 on this investigation's first recording, and up from there.
    pub version: u64,
    /// Where the group was left. A resolved, ignored or regressed group keeps
    /// its state and takes the diagnosis anyway.
    pub group_status: GroupStatusV1,
}

/// One recorded move of a group, for the History tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupTransitionV1 {
    pub id: String,
    /// Absent on the row that records the group's own beginning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_status: Option<GroupStatusV1>,
    pub to_status: GroupStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<GroupChangeReasonV1>,
    /// A role rather than a person: `ingest`, `agent`, `investigation` or
    /// `console`. The console hands this worker no user identity, and a
    /// worker uuid here would read as one.
    pub actor: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupHistoryRequestV1 {
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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupHistoryResponseV1 {
    /// Newest first.
    pub transitions: Vec<GroupTransitionV1>,
    pub total: u64,
}

/// Every diagnosis recorded against one group, across investigations.
///
/// Separate from `groups::get`, which carries only the one in force: the
/// history is what the Diagnosis tab opens, and most readers never ask for
/// it.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosesListRequestV1 {
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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosesListResponseV1 {
    /// Newest first: the head is the one in force.
    pub diagnoses: Vec<DiagnosisRecordV1>,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationGetRequestV1 {
    pub investigation_id: String,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationGetResponseV1 {
    pub investigation: InvestigationSummaryV1,
    /// Newest first.
    pub diagnoses: Vec<DiagnosisRecordV1>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationsListRequestV1 {
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub status: Option<Vec<InvestigationStatusV1>>,
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

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationsListResponseV1 {
    pub investigations: Vec<InvestigationSummaryV1>,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationCancelRequestV1 {
    pub investigation_id: String,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

/// What an investigation changed to, for the console and for siblings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationChangedEventV1 {
    pub op: InvestigationChangedOpV1,
    pub investigation_id: String,
    pub group_id: String,
    pub session_id: String,
    pub status: InvestigationStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationChangedOpV1 {
    Created,
    /// A diagnosis was recorded in it.
    Recorded,
    Finished,
}

/// The harness doorbell. Only a wake-up: the truth is re-read from
/// `harness::status`, never taken from this payload.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct TurnCompletedEventV1 {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub terminal: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DoorbellResponseV1 {
    /// Whether this doorbell named an investigation of this worker's.
    pub handled: bool,
}

// ── the redacted windows onto the live engine ────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceGetRequestV1 {
    pub trace_id: String,
    /// Also return the flat span list, for a trace whose shape matters more
    /// than its tree.
    #[serde(default)]
    pub include_spans: bool,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceGetResponseV1 {
    /// Null when the trace has already left the engine's ring: the frozen
    /// evidence is then all there is.
    pub trace: Option<serde_json::Value>,
    /// Values rewritten before the response left this worker.
    pub redactions: u64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogsListRequestV1 {
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    /// `error`, `warn`, `info`, … Matched by the engine.
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Injected by the iii engine. Accepted on the wire, absent from the
    /// published schema, and never part of a request's meaning.
    #[serde(default)]
    #[schemars(skip)]
    pub _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogsListResponseV1 {
    pub logs: Vec<serde_json::Value>,
    pub redactions: u64,
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
