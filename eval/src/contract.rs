//! Public contracts of the session monitor: configuration, analysis records,
//! captured evidence, triage, suggestions and E2E validation links.

use std::collections::BTreeMap;

use harness::functions::metrics::SessionMetricsResponseV1;
use harness::types::model::ThinkingLevel;
use harness::types::turn::TurnStatus;
use judge_contract::{Answer, Stats};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::EvalError;

pub const RECORD_SCHEMA_VERSION: u32 = 1;
/// Identity of the deterministic detectors and evidence-selection rules.
pub const RULES_VERSION: &str = "session-monitor-rules/1";
pub const DEFAULT_LIST_LIMIT: u32 = 50;
pub const MAX_LIST_LIMIT: u32 = 200;
pub const MAX_SUGGESTIONS: usize = 3;
/// Code references one suggestion may carry.
pub const MAX_CODE_REFS: usize = 8;

/// The Harness model the user chose for investigations. Credentials stay in
/// the provider worker; this request never carries a key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorModelV1 {
    pub model: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// Provider-native options, namespaced by the selected provider id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureRequestV1 {
    pub enabled: bool,
    pub model: MonitorModelV1,
    /// Absolute path of the codebase directory the investigation reads and
    /// runs in (normally the iii workers repository, checked out on the
    /// monitor's host); absent means no code access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_repository: Option<String>,
    /// Stamped by the engine on every invocation; callers omit it. Listed
    /// so the closed schema still accepts the engine's metadata.
    #[serde(rename = "_caller_worker_id", default, skip_serializing)]
    pub caller_worker_id: Option<String>,
}

impl ConfigureRequestV1 {
    pub fn validate(&self) -> Result<(), EvalError> {
        let model = &self.model;
        if model.model.trim().is_empty() || model.provider.trim().is_empty() {
            return Err(EvalError::InvalidRequest(
                "model.model and model.provider must name a catalog model".into(),
            ));
        }
        if let Some(options) = &model.provider_options {
            if options.keys().any(|key| key != &model.provider) {
                return Err(EvalError::InvalidRequest(format!(
                    "model.provider_options may only hold the `{}` namespace",
                    model.provider
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorConfigV1 {
    pub enabled: bool,
    pub model: MonitorModelV1,
    /// The codebase directory the investigation reads (see
    /// `ConfigureRequestV1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_repository: Option<String>,
    /// SHA-256 of the effective configuration; analyses keep the revision
    /// they were admitted with.
    pub revision: String,
    pub updated_at: i64,
    /// When automatic observation last went from paused to on; analyses use
    /// it to bound which earlier turns they may report. Unchanged by edits
    /// that keep observation on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_since: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorStateResponseV1 {
    /// `null` until the user configures the monitor.
    pub config: Option<MonitorConfigV1>,
    /// False when the `harness::turn-completed` binding was refused locally:
    /// automatic observation is unavailable even if `config.enabled` is true.
    /// The SDK does not wait for the engine's acknowledgement, so true means
    /// the binding was requested, not confirmed.
    pub observer_bound: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observer_error: Option<String>,
    /// The latest turn not admitted because the monitor was at capacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rejection: Option<CapacityRejectionV1>,
    /// Present when the request asked to check the triage provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageAvailabilityV1>,
    /// The limits this version enforces, so the console never restates them.
    pub limits: MonitorLimitsV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorLimitsV1 {
    /// Whole-analysis budget from admission, queue wait included.
    pub analysis_budget_ms: u64,
    pub judge_timeout_ms: u64,
    /// Serialized JSON shown to a model (bytes, not tokens).
    pub model_context_bytes: u64,
    pub assets_bytes: u64,
    pub investigation_max_turns: u32,
    pub investigation_max_output_tokens: u64,
    pub investigation_max_total_tokens: u64,
    /// Generate-step cap of an investigation that has code access.
    pub investigation_code_max_turns: u32,
    /// Total-token cap of an investigation that has code access.
    pub investigation_code_max_total_tokens: u64,
    pub queue_concurrency: u32,
    pub max_active_analyses: u32,
    pub low_confidence: f64,
    pub audit_sample_percent: u32,
    pub retention_days: u32,
    pub retention_max_terminal: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapacityRejectionV1 {
    pub session_id: String,
    pub turn_id: String,
    pub at: i64,
}

/// What `judge::models::list` answered for the triage provider. The key
/// stays in the provider; this only reports whether it answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriageAvailabilityV1 {
    pub provider: String,
    pub available: bool,
    /// The provider's error code (`missing_key`, `provider_unavailable`…),
    /// or `unreachable` when the hub did not answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub models: Vec<String>,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvalStatusV1 {
    Queued,
    Collecting,
    Judging,
    Investigating,
    Completed,
    Failed,
    Cancelled,
}

impl EvalStatusV1 {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisOriginV1 {
    Automatic,
    Manual,
}

/// The stage that failed and why. `stage` is the status the analysis was in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailureV1 {
    pub stage: EvalStatusV1,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoverageLevelV1 {
    /// Every in-scope transcript was read and the model context holds every
    /// diagnostic's evidence.
    Complete,
    /// Previews omit or shorten entries; the full transcripts were read.
    Partial,
    /// Required evidence did not fit; no conclusion of healthy behavior.
    Insufficient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutingReasonV1 {
    Diagnostics,
    NeedsInvestigation,
    InsufficientEvidence,
    LowConfidence,
    CoverageInsufficient,
    AuditSample,
    ManualRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutingV1 {
    pub investigate: bool,
    pub reasons: Vec<RoutingReasonV1>,
}

/// A Jev call started by this analysis. Persisted before the call so a
/// restart can tell a lost answer from one never requested.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JudgeCallV1 {
    pub request_id: String,
    pub started_at: i64,
    pub deadline: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalystRefV1 {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub sent_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCountersV1 {
    pub sessions: u32,
    pub entries: u32,
    pub diagnostics: u32,
    pub suggestions: u32,
    pub rejected_suggestions: u32,
    pub validations: u32,
}

/// The compact analysis record listed by `eval::list`; evidence and model
/// outputs live separately in the analysis assets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisRecordV1 {
    pub schema_version: u32,
    pub evaluation_id: String,
    pub observation_key: String,
    pub origin: AnalysisOriginV1,
    pub session_id: String,
    pub turn_id: String,
    /// The session's title, once collection has read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    /// Frozen at admission: later configuration changes never alter it.
    pub model: MonitorModelV1,
    /// Frozen at admission with the model: the directory the investigation
    /// reads and runs in, absent when code access was off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_root: Option<String>,
    pub config_revision: String,
    pub rules_version: String,
    pub criteria_version: String,
    pub status: EvalStatusV1,
    pub step: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub deadline: i64,
    /// Earlier root turns join the window only if they started after this
    /// instant: the monitor was observing then. Turns from before the
    /// monitor was enabled are never reported as new.
    pub observe_since: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub counters: AnalysisCountersV1,
    /// When each status began, oldest first.
    pub stages: Vec<StageTimeV1>,
    /// The monitor's own consumption, apart from the observed task.
    pub usage: MonitorUsageV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageLevelV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingV1>,
    /// Why collection is still waiting (descendants running, metrics
    /// incomplete).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_call: Option<JudgeCallV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyst: Option<AnalystRefV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageTimeV1 {
    pub status: EvalStatusV1,
    pub at: i64,
}

/// Known usage only: absent counters are unknown, never zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorUsageV1 {
    pub judge_calls: u32,
    pub judge_input_tokens: u64,
    pub judge_output_tokens: u64,
    /// False when Jev reported incomplete usage or failed.
    pub judge_usage_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_cost_usd: Option<f64>,
}

/// A transcript entry, addressed the way `session::messages` returns it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntryRefV1 {
    pub session_id: String,
    pub entry_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CorrelationV1 {
    /// A `registry-changed` notice reached the model between the sources and
    /// the repeated call. Temporal correlation, not a proven cause.
    HarnessNoticeCorrelated,
    Unknown,
}

/// One deterministic observation. Model judgments never remove it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticV1 {
    pub rule_id: String,
    pub rule_version: String,
    /// Rule, session, turn and the call ids of this occurrence.
    pub fingerprint: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub target: String,
    pub observation: String,
    pub correlation: CorrelationV1,
    /// Calls, results and notices, in transcript order.
    pub evidence: Vec<EntryRefV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionEvidenceV1 {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    pub depth: u32,
    /// Out-of-scope descendants belong to earlier turns and are not read.
    pub in_scope: bool,
    pub entries: u32,
    /// SHA-256 of the collected JSON entries (not of the stored bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_sha256: Option<String>,
    /// Masked, shortened entries shown to the models.
    pub preview: Vec<Value>,
    pub omitted_entries: u32,
    /// Preview entries whose strings were truncated or whose secrets were
    /// masked.
    pub reduced_entries: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverageV1 {
    pub level: CoverageLevelV1,
    pub sessions_in_scope: u32,
    pub sessions_out_of_scope: u32,
    pub entries_read: u32,
    /// Diagnostics shown to the models; all of them stay in the snapshot.
    pub diagnostics_in_context: u32,
    pub context_bytes: u32,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricsScopeV1 {
    /// Cumulative over the root and every descendant, all turns.
    SessionTree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotV1 {
    pub captured_at: i64,
    pub rules_version: String,
    pub source_session_id: String,
    pub source_turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    pub source_status: TurnStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_result_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_provider: Option<String>,
    /// The observed turn plus earlier root turns no analysis covered yet.
    pub window_turn_ids: Vec<String>,
    pub sessions: Vec<SessionEvidenceV1>,
    pub metrics_scope: MetricsScopeV1,
    pub metrics: SessionMetricsResponseV1,
    pub diagnostics: Vec<DiagnosticV1>,
    /// Protocol probes that failed as expected and were not reported.
    pub excluded_probes: Vec<ExcludedProbeV1>,
    pub coverage: CoverageV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExcludedProbeV1 {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub target: String,
    pub code: String,
    pub calls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriageV1 {
    pub provider: String,
    pub request_id: String,
    /// The model the provider reports, not the configured default.
    pub model: String,
    pub criteria_version: String,
    pub answers: BTreeMap<String, Answer>,
    pub stats: Stats,
    pub completed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriageFailureV1 {
    pub request_id: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// Known usage of the failed call, when the provider returned it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
    pub message: String,
}

/// Lines of one file of the code directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeRefV1 {
    /// Relative to the code directory, e.g. `harness/src/registry.rs` or
    /// `context-manager/src/prune.rs`; never absolute and never with `..`.
    pub path: String,
    /// First line, 1-based and inclusive.
    pub line_from: u32,
    /// Last line, inclusive; at most the file's line count.
    pub line_to: u32,
}

/// The E2E plan a suggestion must carry. A reviewer can run it without the
/// LLM that wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationPlanV1 {
    /// An existing harness-e2e scenario, or null when a new case is needed.
    pub scenario_id: Option<String>,
    pub reproduction: String,
    /// Task-correctness invariants checked by an independent evaluator.
    pub invariants: Vec<String>,
    /// The effort metric compared between baseline and candidate.
    pub primary_metric: String,
    pub expectation: String,
    pub non_regression_controls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionV1 {
    pub title: String,
    pub observation: String,
    pub hypothesis: String,
    pub harness_component: String,
    pub proposed_change: String,
    pub expected_effect: String,
    pub evidence: Vec<EntryRefV1>,
    /// Code the suggestion rests on, as read in the code directory; only with
    /// code access. At most 8; each is checked against the directory.
    #[serde(default)]
    #[schemars(length(max = 8))]
    pub code_refs: Vec<CodeRefV1>,
    pub limitations: String,
    pub validation: ValidationPlanV1,
}

/// The only shape the investigating LLM may return; an empty list is a
/// valid answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationOutputV1 {
    #[schemars(length(max = 3))]
    pub suggestions: Vec<SuggestionV1>,
    /// The analyst's reading of each deterministic signal. It explains a
    /// signal; it never removes one.
    #[serde(default)]
    pub signal_assessments: Vec<SignalAssessmentV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalVerdictV1 {
    LikelyExpected,
    WorthChanging,
    Unclear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalAssessmentV1 {
    /// The `fingerprint` of a diagnostic in the evidence.
    pub fingerprint: String,
    pub verdict: SignalVerdictV1,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectedSuggestionV1 {
    pub index: usize,
    pub title: String,
    pub reasons: Vec<String>,
}

/// The investigation and its own consumption, kept apart from the observed
/// task's metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvestigationV1 {
    pub session_id: String,
    pub turn_id: String,
    pub requested_model: String,
    pub requested_provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_provider: Option<String>,
    pub suggestions: Vec<SuggestionV1>,
    pub rejected: Vec<RejectedSuggestionV1>,
    pub signal_assessments: Vec<SignalAssessmentV1>,
    /// The directory the analyst read and ran in; absent when access was off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<SessionMetricsResponseV1>,
    pub completed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct E2eReportRefV1 {
    pub scenario_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct E2eMeasureV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average: Option<f64>,
    /// Runs that reported this measure; fewer than `run_count` means some
    /// runs reported nothing (never counted as zero).
    pub samples: u32,
}

/// One scenario's identity and the execution's own averages, as the E2E
/// computed them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct E2eScenarioV1 {
    pub scenario_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_fingerprint: Option<String>,
    pub run_count: u32,
    /// `function_calls`, `function_call_errors`, `tokens`,
    /// `duration_seconds` and `cost_usd`, when reported.
    pub measures: BTreeMap<String, E2eMeasureV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct E2eAssessmentsV1 {
    pub passed: u32,
    pub total: u32,
}

/// What `e2e::dashboard::execution-get` reported at lookup time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct E2eExecutionRefV1 {
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<String>,
    /// True only when the execution reported retained reports.
    pub reports_available: bool,
    pub reports: Vec<E2eReportRefV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2e_revision: Option<String>,
    pub scenarios: Vec<E2eScenarioV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessments: Option<E2eAssessmentsV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparabilityCheckV1 {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
    pub matches: bool,
}

/// The conditions that must match for a fair comparison; only the Harness
/// version is meant to differ.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparabilityV1 {
    pub comparable: bool,
    pub checks: Vec<ComparabilityCheckV1>,
}

/// Baseline and candidate executions linked to one suggestion. A link is a
/// reference, not a verdict: the improvement claim belongs to the E2E
/// comparison and its criteria.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationLinkV1 {
    pub suggestion_index: usize,
    pub baseline: E2eExecutionRefV1,
    pub candidate: E2eExecutionRefV1,
    pub comparability: ComparabilityV1,
    pub attached_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisAssetsV1 {
    pub evaluation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage_failure: Option<TriageFailureV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub investigation: Option<InvestigationV1>,
    #[serde(default)]
    pub validations: Vec<ValidationLinkV1>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct MonitorStateRequestV1 {
    /// Also ask the triage provider whether it answers (`judge::models::list`).
    #[serde(default)]
    pub check_providers: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AnalyzeSessionRequestV1 {
    pub session_id: String,
    /// Explicitly create another analysis of an already analyzed turn.
    #[serde(default)]
    pub reanalyze: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeSessionResponseV1 {
    pub evaluation_id: String,
    pub status: EvalStatusV1,
    /// True when an existing analysis of the same session turn was returned.
    pub reused: bool,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct EvalListRequestV1 {
    #[serde(default)]
    pub limit: Option<u32>,
}

impl EvalListRequestV1 {
    pub fn normalized_limit(&self) -> Result<usize, EvalError> {
        let limit = self.limit.unwrap_or(DEFAULT_LIST_LIMIT);
        if !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(EvalError::InvalidRequest(format!(
                "limit must be between 1 and {MAX_LIST_LIMIT}"
            )));
        }
        Ok(limit as usize)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalListResponseV1 {
    pub evaluations: Vec<AnalysisRecordV1>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EvaluationIdRequestV1 {
    pub evaluation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalResultResponseV1 {
    pub record: AnalysisRecordV1,
    pub assets: AnalysisAssetsV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalCancelResponseV1 {
    pub cancelled: bool,
    pub status: EvalStatusV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalDeleteResponseV1 {
    pub deleted: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AttachValidationRequestV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
    pub baseline_execution_id: String,
    pub candidate_execution_id: String,
    /// Look both executions up and return the link without saving it.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachValidationResponseV1 {
    pub link: ValidationLinkV1,
    pub saved: bool,
}

/// Asks Jev to pick, among the existing E2E executions, the baseline and the
/// candidate that fit one suggestion's validation plan. Nothing is attached.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProposeValidationRequestV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOutcomeV1 {
    /// Jev chose a pair; `proposal` holds it.
    Proposed,
    /// Jev judged that no listed pair compares the Harness without and with
    /// the change.
    NoneFits,
    /// Code found no pair of runs of the same case, so Jev was not asked.
    NoComparablePair,
}

/// Jev's pick. `confidence` describes Jev's distribution over the offered
/// pairs, not the probability that the pair is right: check the runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationProposalV1 {
    pub baseline_execution_id: String,
    pub candidate_execution_id: String,
    pub confidence: f64,
    /// `confidence` is below the monitor's low-confidence threshold.
    pub low_confidence: bool,
    /// The recorded stack difference computed in code, as Jev saw it:
    /// "recorded stacks identical", "recorded stack differs: ..." or
    /// "recorded stack unknown".
    pub stack_note: String,
}

/// Another offered pair Jev found likely; `probability` is Jev's share for
/// it among the offered options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalAlternativeV1 {
    pub baseline_execution_id: String,
    pub candidate_execution_id: String,
    pub probability: f64,
    pub stack_note: String,
}

/// The model that answered and what the call consumed; the usage is also
/// added to the analysis record.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalJevV1 {
    pub model: String,
    pub request_id: String,
    pub stats: Stats,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeValidationResponseV1 {
    pub outcome: ProposalOutcomeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ValidationProposalV1>,
    /// Executions the E2E returned (its 100 detailed executions at most).
    pub runs_listed: u32,
    /// Eligible runs the pairs were built from: `passed` or `failed`, and
    /// including the plan's scenario when it names one.
    pub runs_considered: u32,
    /// Ordered (baseline, candidate) pairs offered to Jev: the most recent
    /// comparable ones, at most 60.
    pub pairs_considered: u32,
    /// Comparable pairs left out because older pairs were kept instead.
    pub pairs_dropped: u32,
    /// Runs left out, by reason: the E2E status for runs that are not
    /// `passed` or `failed` (`incomplete`, `technical_failed`, ...),
    /// `other_scenario` and `no_id`.
    pub excluded: BTreeMap<String, u32>,
    /// Up to three other offered pairs with at least 5% of Jev's
    /// probability, most likely first; empty when Jev was not asked.
    #[serde(default)]
    pub alternatives: Vec<ProposalAlternativeV1>,
    /// Absent when Jev was not asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev: Option<ProposalJevV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepRequestV1 {
    pub evaluation_id: String,
    pub step: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepResponseV1 {
    pub skipped: bool,
    pub status: EvalStatusV1,
}

/// `harness::turn-completed`; extra fields are tolerated.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct WakeEventV1 {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub parent: Option<Value>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WakeOutcomeV1 {
    Ignored,
    Progress,
    Descendant,
    MonitorSession,
    Disabled,
    Stale,
    Admitted,
    Reused,
    AtCapacity,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WakeResponseV1 {
    pub outcome: WakeOutcomeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct SweepEventV1 {
    #[serde(default)]
    pub scheduled_at: Option<i64>,
    #[serde(default)]
    pub scheduled_time: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SweepResponseV1 {
    pub requeued: u64,
    pub expired: u64,
    pub reconciled: u64,
    pub retained_deleted: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn configure_accepts_engine_metadata_and_rejects_credentials() {
        let request: ConfigureRequestV1 = serde_json::from_value(json!({
            "enabled": false,
            "model": {"model": "m", "provider": "p"},
            "_caller_worker_id": "console"
        }))
        .unwrap();
        assert!(request.validate().is_ok());
        assert!(serde_json::from_value::<ConfigureRequestV1>(json!({
            "enabled": true,
            "model": {"model": "m", "provider": "p"},
            "api_key": "secret"
        }))
        .is_err());
        assert!(serde_json::from_value::<ConfigureRequestV1>(json!({
            "enabled": true,
            "model": {"model": "m", "provider": "p", "api_key": "secret"}
        }))
        .is_err());
    }

    #[test]
    fn configure_requires_an_explicit_model_and_provider_namespace() {
        let mut request: ConfigureRequestV1 = serde_json::from_value(json!({
            "enabled": true,
            "model": {"model": "m", "provider": " "}
        }))
        .unwrap();
        assert!(request.validate().is_err());
        request.model.provider = "p".into();
        request.model.provider_options = Some(BTreeMap::from([("other".into(), json!({}))]));
        assert!(request.validate().is_err());
        request.model.provider_options = Some(BTreeMap::from([("p".into(), json!({}))]));
        assert!(request.validate().is_ok());
    }

    #[test]
    fn llm_output_rejects_unknown_fields() {
        let output = json!({"suggestions": [], "verdict": "validated"});
        assert!(serde_json::from_value::<InvestigationOutputV1>(output).is_err());
        assert!(
            serde_json::from_value::<InvestigationOutputV1>(json!({"suggestions": []})).is_ok()
        );
        let invented_verdict = json!({"suggestions": [], "signal_assessments": [
            {"fingerprint": "f", "verdict": "proven", "explanation": "x"}]});
        assert!(serde_json::from_value::<InvestigationOutputV1>(invented_verdict).is_err());
    }

    #[test]
    fn list_limit_defaults_and_stays_bounded() {
        assert_eq!(
            EvalListRequestV1::default().normalized_limit().unwrap(),
            DEFAULT_LIST_LIMIT as usize
        );
        assert!(EvalListRequestV1 { limit: Some(0) }
            .normalized_limit()
            .is_err());
        assert!(EvalListRequestV1 {
            limit: Some(MAX_LIST_LIMIT + 1)
        }
        .normalized_limit()
        .is_err());
    }

    #[test]
    fn wake_event_tolerates_additional_fields() {
        let event: WakeEventV1 = serde_json::from_value(json!({
            "session_id": "s", "turn_id": "t", "terminal": true, "timestamp": 5,
            "status": "completed", "result": {"ok": true}, "context": {}
        }))
        .unwrap();
        assert!(event.terminal);
        assert_eq!(event.timestamp, Some(5));
    }
}
