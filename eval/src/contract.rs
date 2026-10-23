//! Public contracts of the session monitor: configuration, analysis records,
//! captured evidence, triage and suggestions.

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
    /// US dollars of known investigation cost per UTC day after which no
    /// automatic analysis starts an investigation until the next day; absent
    /// means no cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_cost_cap_usd: Option<f64>,
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
        if self
            .daily_cost_cap_usd
            .is_some_and(|cap| !cap.is_finite() || cap <= 0.0)
        {
            return Err(EvalError::InvalidRequest(
                "daily_cost_cap_usd must be a positive number of US dollars; omit it for no cap"
                    .into(),
            ));
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
    /// The daily cap on known investigation cost (see `ConfigureRequestV1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_cost_cap_usd: Option<f64>,
    /// SHA-256 of the effective configuration; analyses keep the revision
    /// they were admitted with.
    pub revision: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorStateResponseV1 {
    /// `null` until the user configures the monitor.
    pub config: Option<MonitorConfigV1>,
    /// Present when the request asked to check the triage provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageAvailabilityV1>,
    /// What the investigations cost so far, to decide before enabling.
    pub cost: MonitorCostV1,
    /// The limits this version enforces, so the console never restates them.
    pub limits: MonitorLimitsV1,
}

/// The monitor's own spend. Only the investigation's LLM cost is in dollars;
/// Jev's usage is reported in tokens on each record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorCostV1 {
    /// Start (ms since the Unix epoch) of the UTC day the `today_*` values
    /// cover. The monitor has no timezone setting, so a day is a UTC day.
    pub since: i64,
    /// Everything the monitor is known to have spent since `since`: equal to
    /// `today_capture_usd`.
    pub today_usd: f64,
    /// The capture bucket, the only one the cap compares: the larger of the
    /// day's persisted capture spend (deleting an analysis does not give it
    /// back) and the sum of the stored analyses' `usage.llm_cost_usd`. A day
    /// persisted before the buckets counts entirely as capture.
    pub today_capture_usd: f64,
    /// Analyses of the day whose investigation started but reported no cost.
    /// They add nothing to `today_capture_usd`, but their cost is unknown, not zero.
    pub today_unknown: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_usd: Option<f64>,
    /// A cap is set and `today_capture_usd`, plus the median cost of each
    /// investigation still running, reached it: no automatic analysis starts
    /// an investigation until the next UTC day.
    pub capped: bool,
    /// What one investigation has cost, from the history.
    pub per_analysis: AnalysisCostStatsV1,
}

/// Cost of the completed analyses that investigated with the configured
/// model, provider and code access (or lack of it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCostStatsV1 {
    /// Analyses with a known cost; `min`, `median` and `max` come from them.
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Analyses that investigated but reported no cost.
    pub unknown: u32,
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
    pub retention_days: u32,
    pub retention_max_terminal: u32,
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
    NeedsInvestigation,
    /// Legacy: no longer produced; kept so stored records still deserialize.
    Diagnostics,
    /// Legacy: no longer produced.
    InsufficientEvidence,
    /// Legacy: no longer produced.
    LowConfidence,
    /// Legacy: no longer produced.
    CoverageInsufficient,
    /// Legacy: no longer produced.
    AuditSample,
    /// Legacy: no longer produced.
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
    /// The earlier analysis of the same turn this reanalysis was requested
    /// over (same `observation_key`); absent on a first analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// The Harness version the engine reported when a live event admitted the
    /// analysis (the turn just ended on it); absent on a manual analysis, which
    /// may be of an older session, and when the engine did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
    /// Occurrences of each deterministic pattern the collection found, keyed
    /// `<rule_id>:<target>` (a target may itself contain colons). Empty both
    /// when nothing was found and before collection (`coverage` is set once
    /// collection read the evidence).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub signals: BTreeMap<String, u32>,
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

impl DiagnosticV1 {
    /// The recurring pattern this occurrence belongs to, `<rule_id>:<target>`
    /// (a target may itself contain colons): what a release is judged by.
    pub fn pattern(&self) -> String {
        format!("{}:{}", self.rule_id, self.target)
    }
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
    /// The harness-e2e scenario the observed session ran, when it came from
    /// the E2E (`metadata.e2e_scenario`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e2e_scenario: Option<String>,
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
    /// How to reproduce the behavior at the step where it happened, and the
    /// proposed change as edits of what the model saw there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<SuggestionCheckV1>,
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
    /// The scenario's cohort in `plan_execution.measurements.cohorts[]`, when
    /// the execution has exactly one for it. Each value below is absent when
    /// the E2E did not report it, never zero.
    ///
    /// Share of the planned runs that passed (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_rate: Option<f64>,
    /// Mean score of the scored runs (0 to 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_runs: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_runs: Option<u32>,
    /// The cohort's total cost divided by its completed runs; the total also
    /// covers failed attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd_per_run: Option<f64>,
    /// The cohort's total tokens divided by its completed runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens_per_run: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_wall_time_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p50_function_calls: Option<f64>,
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
    /// When the E2E started the execution (ms since the Unix epoch): its
    /// results cannot have existed before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
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
    /// The observed turn's record, copied when the evidence was captured:
    /// what a later fork of the session needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<TurnCaptureV1>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct MonitorStateRequestV1 {
    /// Also ask the triage provider whether it answers (`judge::models::list`).
    #[serde(default)]
    pub check_providers: bool,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct EvalListRequestV1 {
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only the analyses of this observed turn (the `observation_key` of a
    /// record), reanalyses included.
    #[serde(default)]
    pub observation_key: Option<String>,
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

/// Version of the loop cycle record; the cycle scope holds only this shape.
pub const CYCLE_SCHEMA_VERSION: u32 = 1;

/// What started a loop cycle: a free objective, or an existing E2E
/// execution. Both start the same cycle and land in the same history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CycleInputV1 {
    Objective(String),
    ExecutionId(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CycleStatusV1 {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl CycleStatusV1 {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// The cycle status as the shared `eval::step` response reports it.
    pub fn as_step_status(self) -> EvalStatusV1 {
        match self {
            Self::Queued => EvalStatusV1::Queued,
            Self::Running => EvalStatusV1::Collecting,
            Self::Completed => EvalStatusV1::Completed,
            Self::Failed => EvalStatusV1::Failed,
            Self::Cancelled => EvalStatusV1::Cancelled,
        }
    }
}

/// One status change of a cycle, oldest first in `CycleRecordV1::history`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleEventV1 {
    pub at: i64,
    pub status: CycleStatusV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What a finished cycle produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleResultV1 {
    pub summary: String,
    /// The E2E executions the cycle read or started.
    #[serde(default)]
    pub execution_ids: Vec<String>,
}

/// The one record of a loop cycle, whichever input started it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleRecordV1 {
    pub schema_version: u32,
    pub cycle_id: String,
    pub input: CycleInputV1,
    pub status: CycleStatusV1,
    /// Durable step counter; a queued step that does not match is stale.
    pub step: u64,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub history: Vec<CycleEventV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CycleResultV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// The plan frozen before any result, with its `policy_revision`: what
    /// the decision is keyed by. Once set it never changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<CyclePlanV1>,
    /// The decision on the cycle's pair, recorded once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<CycleDecisionV1>,
}

/// How many measured runs each side needs and the inputs they run on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SamplingPlanV1 {
    pub runs_per_side: u32,
    /// The inputs/seeds both sides must run on; empty means only that the
    /// two sides report the same ones.
    #[serde(default)]
    pub seeds: Vec<String>,
}

/// What a cycle commits to before any result is recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CyclePlanV1 {
    pub objective: String,
    /// The metric, its direction, the minimum effect and the scenario.
    pub criterion: CriterionV1,
    /// Correctness conditions the candidate must preserve: one that regresses
    /// rejects the candidate whatever the metric did, one that goes
    /// unmeasured makes the decision inconclusive. With `min_effect` 0 any
    /// loss is a regression.
    #[serde(default)]
    pub preserved: Vec<CriterionV1>,
    /// The controls the cycle runs next to the pair.
    #[serde(default)]
    pub controls: Vec<String>,
    pub sampling: SamplingPlanV1,
    /// The revision of the policy the cycle runs under; both sides must
    /// report it.
    pub policy_revision: String,
    pub frozen_at: i64,
}

/// One side of the compared pair: the E2E execution and what it ran on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionSideV1 {
    pub execution: E2eExecutionRefV1,
    #[serde(default)]
    pub seeds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcomeV1 {
    /// The pair is comparable, every preserved condition held and the metric
    /// moved by the minimum effect the wanted way.
    Accepted,
    /// A preserved condition was lost, or the metric did not improve.
    Rejected,
    /// An incompatible pair, a technical failure or a missing measure.
    Inconclusive,
}

/// The cycle's decision and the evidence it rests on, computed in code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleDecisionV1 {
    pub outcome: DecisionOutcomeV1,
    pub reason: String,
    /// The frozen plan's `policy_revision` the decision was made under.
    pub policy_revision: String,
    pub comparability: ComparabilityV1,
    /// The plan's criterion, judged.
    pub primary: EvidenceV1,
    /// Each preserved condition, judged, in the plan's order.
    pub preserved: Vec<EvidenceV1>,
    pub decided_at: i64,
}

/// Starts a cycle from exactly one of `objective` or `execution_id`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CycleStartRequestV1 {
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub execution_id: Option<String>,
}

impl CycleStartRequestV1 {
    pub fn input(&self) -> Result<CycleInputV1, EvalError> {
        let objective = self
            .objective
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        let execution_id = self
            .execution_id
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        match (objective, execution_id) {
            (Some(objective), None) => Ok(CycleInputV1::Objective(objective.into())),
            (None, Some(execution_id)) => Ok(CycleInputV1::ExecutionId(execution_id.into())),
            _ => Err(EvalError::InvalidRequest(
                "give exactly one of objective or execution_id".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleStartResponseV1 {
    pub cycle_id: String,
    pub status: CycleStatusV1,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CycleIdRequestV1 {
    pub cycle_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CycleListRequestV1 {
    #[serde(default)]
    pub limit: Option<u32>,
}

impl CycleListRequestV1 {
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

/// Every cycle, newest first, whichever input started it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleListResponseV1 {
    pub cycles: Vec<CycleRecordV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleResultResponseV1 {
    pub cycle: CycleRecordV1,
    /// `null` until the cycle completes.
    pub result: Option<CycleResultV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleCancelResponseV1 {
    pub cancelled: bool,
    pub status: CycleStatusV1,
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
    pub retained_deleted: u64,
}

/// An `e2e::dashboard::changed` event. Only the execution it names is read;
/// `execution-get` stays the source of truth. An event naming none runs the
/// checkpointed admission sweep instead.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ExecutionChangedEventV1 {
    #[serde(default)]
    pub execution_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

impl ExecutionChangedEventV1 {
    pub fn execution_id(&self) -> Option<&str> {
        self.execution_id
            .as_deref()
            .or(self.id.as_deref())
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }
}

/// How an admitted execution entered the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionKindV1 {
    /// A normal terminal execution that started its own cycle.
    Cycle,
    /// An execution started to validate a candidate of `cycle_id`: it returns
    /// to that cycle and never starts a new one.
    CandidateValidation,
}

/// The claim of one execution/run/attempt, set once with compare-and-set:
/// redelivery and restart find it and never admit twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionV1 {
    pub admission_key: String,
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    pub kind: AdmissionKindV1,
    pub cycle_id: String,
    pub claimed_at: i64,
}

/// Where the admission sweep resumes: executions that finished before
/// `since` (ms since the Unix epoch) were already looked at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionCheckpointV1 {
    pub since: i64,
    pub updated_at: i64,
}

/// The six classes the deterministic first pass sorts a finding into.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FindingClassV1 {
    Defect,
    ImprovementOpportunity,
    EvaluationGap,
    InfrastructureOrEvaluatorFailure,
    ExpectedBehavior,
    InsufficientEvidence,
}

impl FindingClassV1 {
    /// Only these justify deepening the analysis.
    pub fn is_pertinent(self) -> bool {
        matches!(
            self,
            Self::Defect | Self::ImprovementOpportunity | Self::EvaluationGap
        )
    }
}

/// One classified first-pass finding; `fingerprint` makes re-import a no-op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingV1 {
    pub fingerprint: String,
    pub class: FindingClassV1,
    /// The diagnostics rule or execution signal it comes from.
    pub signal: String,
    pub observation: String,
}

/// Identity of the imported execution. `None` is unknown, never assumed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceIdentityV1 {
    pub scenario_digest: Option<String>,
    pub contract_digest: Option<String>,
    pub artifacts_digest: Option<String>,
    pub stack_digest: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub revision: Option<String>,
}

/// The native report bundle of an admitted execution, copied into the loop
/// scope at admission so analysis survives teardown and source retention,
/// plus the deterministic first pass computed from this copy only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidenceV1 {
    pub admission_key: String,
    pub execution_id: String,
    pub imported_at: i64,
    pub bundle_digest: String,
    pub bundle: Value,
    pub identity: EvidenceIdentityV1,
    #[serde(default)]
    pub analyzed: bool,
    #[serde(default)]
    pub findings: Vec<FindingV1>,
    /// What the stored copy could not tell.
    #[serde(default)]
    pub unknowns: Vec<String>,
    /// True only when every child transcript was preserved in the copy.
    #[serde(default)]
    pub child_transcripts_complete: bool,
    /// Set only when a pertinent finding exists.
    #[serde(default)]
    pub deepen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAdmissionResponseV1 {
    /// True only when this call claimed the execution.
    pub admitted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Comparison: the evidence and outcome of a baseline/candidate pair
// ---------------------------------------------------------------------------

/// What a validation measures. Every metric but `signal_per_run` is the mean
/// over the runs of the scenario the infrastructure did not break
/// (`pass_rate` is the share that passed, an unfinished run counting as not
/// passed; `duration` is the wall time in milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CriterionMetricV1 {
    /// Occurrences of one of the suggestion's patterns per run, counted by
    /// the deterministic detectors over each run's transcript.
    SignalPerRun,
    PassRate,
    CostUsd,
    Duration,
    Tokens,
    FunctionCalls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectionV1 {
    Decrease,
    Increase,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CriterionV1 {
    pub metric: CriterionMetricV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    pub direction: DirectionV1,
    pub min_effect: f64,
    pub min_runs: u32,
    /// The scenario whose runs are measured.
    pub scenario_id: String,
    pub registered_at: i64,
    pub registered_by: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOutcomeV1 {
    /// The candidate moved the metric by at least `min_effect` in the wanted
    /// direction, with `min_runs` completed runs each side.
    ValidatedImprovement,
    /// The metric moved less than `min_effect`.
    NoImprovement,
    /// The metric moved by at least `min_effect` the wrong way.
    Regression,
    /// Too few completed runs, missing data or no criterion.
    Inconclusive,
}

/// One run of the scenario as the E2E reported it. Runs the infrastructure
/// broke are listed with the E2E's own words, never counted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRunV1 {
    pub run_id: String,
    /// The E2E reported the run complete with a valid technical outcome.
    pub completed: bool,
    /// `completion` as the E2E reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<String>,
    /// `technical` as the E2E reported it; anything but `valid` is an
    /// infrastructure failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<String>,
    /// `passed` or `failed`, the run's task outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_time_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_calls: Option<f64>,
    /// Occurrences of each of the row's patterns the detectors found in this
    /// run's transcript (0 is a measurement); absent when the report held no
    /// transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals: Option<BTreeMap<String, u32>>,
}

impl EvidenceRunV1 {
    /// The run says something about the Harness: its technical outcome is
    /// valid, finished or not. One that ran out of steps or gave up is the
    /// Harness's result; a technical failure is the infrastructure's.
    pub fn measurable(&self) -> bool {
        self.technical.as_deref() == Some("valid")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSideV1 {
    pub execution_id: String,
    /// Every run of the scenario the execution holds.
    pub runs: Vec<EvidenceRunV1>,
    /// Measurable runs (see `EvidenceRunV1::measurable`) with a value of the
    /// criterion's metric (every measurable run while there is no criterion).
    pub n: u32,
    /// Mean of the criterion's metric over those runs; absent without a
    /// criterion or without such runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<f64>,
}

/// Computed in code from the attached pair's runs, never by a model. It is
/// what the person's verdict is shown next to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceV1 {
    pub scenario_id: String,
    pub computed_at: i64,
    pub baseline: EvidenceSideV1,
    pub candidate: EvidenceSideV1,
    pub computed_outcome: ValidationOutcomeV1,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Suggestion check at the decision point
// ---------------------------------------------------------------------------

/// How a suggestion is checked at the step where its behavior happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionCheckV1 {
    /// The assistant entry where the behavior happened (`…_<step>_assistant`),
    /// one of the suggestion's `evidence` entries.
    pub decision_point: String,
    pub signal: SignalV1,
    /// The proposed change as edits of what the model saw before that step;
    /// empty when it cannot be expressed as text.
    #[serde(default)]
    pub change: Vec<ChangeEditV1>,
}

/// How to recognize the behavior in one reply: exactly one of a rule computed
/// in code or a yes/no question Jev answers about the reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<SignalRuleV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalRuleV1 {
    /// The reply asks `engine::functions::info` only for contracts an earlier
    /// result in the context already supplied.
    ContractRediscovery,
    /// The reply repeats the last call that failed: same target, equal payload.
    RepeatedErrorCall,
}

/// One edit of what the model saw. `target` is an entry id of the
/// conversation before the decision point, or `system_prompt`. An edit either
/// replaces `find` with `replace` in the target's text or removes the entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeEditV1 {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
}

/// The observed turn's record as the Harness stored it, which keeps only a
/// session's latest turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnCaptureV1 {
    pub turn_id: String,
    /// The Harness's `TurnOptions`, unmodified.
    pub options: Value,
    /// The whole `TurnRecord` as read when the evidence was captured (it
    /// repeats `options`): what a later fork of the session needs, since the
    /// Harness overwrites it with the next turn. Absent when it would have
    /// taken the assets over their size limit (`record_omitted` says so) and
    /// on analyses made before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Value>,
    /// Why `record` was not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_omitted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_digest: Option<String>,
    /// Tokens `pre_generate` hooks added at the turn's last step: above zero,
    /// the system prompt the model saw is not stored anywhere.
    #[serde(default)]
    pub hook_guidance_tokens: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn routing_recorded_under_the_earlier_rules_still_reads() {
        let routing: RoutingV1 = serde_json::from_value(json!({
            "investigate": true,
            "reasons": ["diagnostics", "needs_investigation", "insufficient_evidence",
                "low_confidence", "coverage_insufficient", "audit_sample", "manual_request"]
        }))
        .unwrap();
        assert_eq!(routing.reasons.len(), 7);
    }

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
    fn the_daily_cost_cap_must_be_a_positive_number() {
        let request = |cap: Option<f64>| ConfigureRequestV1 {
            enabled: true,
            model: MonitorModelV1 {
                model: "m".into(),
                provider: "p".into(),
                thinking_level: None,
                provider_options: None,
            },
            code_repository: None,
            daily_cost_cap_usd: cap,
            caller_worker_id: None,
        };
        assert!(request(None).validate().is_ok());
        assert!(request(Some(0.01)).validate().is_ok());
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(request(Some(invalid)).validate().is_err(), "{invalid}");
        }
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
        assert!(EvalListRequestV1 {
            limit: Some(0),
            ..Default::default()
        }
        .normalized_limit()
        .is_err());
        assert!(EvalListRequestV1 {
            limit: Some(MAX_LIST_LIMIT + 1),
            ..Default::default()
        }
        .normalized_limit()
        .is_err());
    }
}
