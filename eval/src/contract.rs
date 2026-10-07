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
    /// US dollars of known investigation cost per UTC day after which
    /// automatic observation admits nothing until the next day; absent means
    /// no cap. A manual `eval::analyze-session` is never refused by it.
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
    /// The latest turn not admitted: the monitor was at capacity or the daily
    /// cost cap was reached (`reason`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rejection: Option<CapacityRejectionV1>,
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
    /// Everything the monitor is known to have spent since `since`:
    /// `today_capture_usd` + `today_replay_usd`. The cap does not compare this.
    pub today_usd: f64,
    /// The capture bucket, the only one the cap compares: the larger of the
    /// day's persisted capture spend (deleting an analysis does not give it
    /// back) and the sum of the stored analyses' `usage.llm_cost_usd`. A day
    /// persisted before the buckets counts entirely as capture.
    pub today_capture_usd: f64,
    /// The replay bucket: the known cost of the day's `eval::reproduce`
    /// samples. Never capped.
    pub today_replay_usd: f64,
    /// Replay samples of the day that came back without a cost. They add
    /// nothing to `today_replay_usd`, but their cost is unknown, not zero.
    pub today_replay_unknown: u32,
    /// Analyses of the day whose investigation started but reported no cost.
    /// They add nothing to `today_capture_usd`, but their cost is unknown, not zero.
    pub today_unknown: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap_usd: Option<f64>,
    /// A cap is set and `today_capture_usd`, plus the median cost of each
    /// investigation still running, reached it: automatic observation admits
    /// and investigates nothing until the next UTC day.
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReasonV1 {
    /// The unfinished-analyses cap was reached.
    #[default]
    AtCapacity,
    /// The day's known cost reached `daily_cost_cap_usd`.
    CostCap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapacityRejectionV1 {
    pub session_id: String,
    pub turn_id: String,
    pub at: i64,
    /// Why the turn was not admitted; rows saved before the cost cap existed
    /// read as `at_capacity`.
    #[serde(default)]
    pub reason: RejectionReasonV1,
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
    /// The observed turn's options, copied when the evidence was captured:
    /// what a reproduction rebuilds the request from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<TurnCaptureV1>,
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
    /// One row per suggestion, in suggestion order: what people decided about
    /// it. A suggestion nobody acted on reads as `new`.
    #[serde(default)]
    pub reviews: Vec<SuggestionReviewV1>,
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
    // The day's known cost reached `daily_cost_cap_usd`; a plain comment so
    // the schema stays a flat string enum.
    CostCap,
    // Automatic observation covers only the user's chats (session kind
    // `user`); E2E runs and automations are analyzed by hand.
    NotUserChat,
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

// ---------------------------------------------------------------------------
// Review: what people decide about a suggestion
// ---------------------------------------------------------------------------

/// Where a suggestion stands. Only people move it: the monitor never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatusV1 {
    New,
    Accepted,
    InProgress,
    Shipped,
    Rejected,
    Duplicate,
}

/// One change of status, with who made it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEventV1 {
    pub status: LifecycleStatusV1,
    pub at: i64,
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleV1 {
    pub status: LifecycleStatusV1,
    /// The pull request that implements it (`in_progress`, `shipped`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// The Harness version that shipped it, a semantic version like `1.8.43`;
    /// the recurrence query splits the analyses at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Why it was rejected (`rejected`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What it duplicates: a pull request, or another `<evaluation_id>:<index>`
    /// (`duplicate`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    /// Every change of status, oldest first; empty while `new`.
    pub history: Vec<LifecycleEventV1>,
}

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

/// What a person commits to before the results exist.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CriterionInputV1 {
    pub metric: CriterionMetricV1,
    /// `signal_per_run` only: one of the row's `patterns`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// The change that counts as an effect, wanted when the candidate moves
    /// the baseline mean in this direction.
    pub direction: DirectionV1,
    /// The smallest change of the baseline mean that counts, as an absolute
    /// difference in the metric's own unit: signals per run, USD, seconds,
    /// calls or tokens; percentage points (0-100) for `pass_rate`. Above 0.
    pub min_effect: f64,
    /// Completed runs each side needs for a verdict of improvement (1-20).
    pub min_runs: u32,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationRunStateV1 {
    /// Recorded before the executions are requested.
    Starting,
    /// Both executions were requested; the sweep waits for them.
    Running,
    /// Both ended; the pair is about to be attached.
    Finished,
    /// The pair is attached to the analysis and its evidence computed.
    Attached,
    Failed,
}

/// The baseline and candidate E2E executions this worker started for a
/// suggestion (`eval::start-validation`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationRunV1 {
    /// Absent until the E2E accepted the execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_execution_id: Option<String>,
    /// The full commits the Harness was built from.
    pub baseline_commit: String,
    pub candidate_commit: String,
    pub scenario_id: String,
    /// Runs requested for each side.
    pub runs: u32,
    pub model: String,
    pub provider: String,
    pub started_at: i64,
    /// When both executions were seen ended; the sweep stops trying to attach
    /// the pair some time after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub state: ValidationRunStateV1,
    /// Why it failed, or why attaching is being retried; starts with a stable
    /// code (`e2e_unavailable:`, `e2e_busy:`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

/// The person's judgment, kept with the criterion it was made against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerdictV1 {
    pub outcome: ValidationOutcomeV1,
    pub rationale: String,
    /// The non-regression controls of the plan that were checked.
    pub controls_checked: Vec<String>,
    pub by: String,
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion_snapshot: Option<CriterionV1>,
}

/// Everything people decided about one suggestion, in the `eval_suggestion`
/// state scope. The title, component and patterns are copied from the
/// suggestion so the row outlives the analysis's retention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestionReviewV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
    pub title: String,
    pub harness_component: String,
    /// The scenario the suggestion's validation plan names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario_id: Option<String>,
    /// `<rule_id>:<target>` of the diagnostics the suggestion's evidence
    /// cites: what a validation counts and a release is judged by.
    pub patterns: Vec<String>,
    pub lifecycle: LifecycleV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<CriterionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<ValidationRunV1>,
    /// When the first execution this worker started for the suggestion began,
    /// kept when a new start replaces `run`: results of it may have been seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictV1>,
    /// Replays of the decision point, oldest first: the base and the tested
    /// changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reproductions: Vec<ReproductionV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewActionV1 {
    /// Move the suggestion to `status`.
    SetLifecycle,
    /// Register the criterion a validation is judged by, before its results.
    SetCriterion,
    /// Record the person's verdict.
    SetVerdict,
}

/// `eval::review`. The fields of the other actions are ignored. The author is
/// the caller's identity when the engine stamps one, else `by`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReviewRequestV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
    pub action: ReviewActionV1,
    /// `set_lifecycle`: the status to move to (never back to `new`; a shipped
    /// suggestion stays shipped, to complete its `pr` or `version`).
    /// `shipped` needs a `pr` (here or already recorded), `rejected` a
    /// `reason`, `duplicate` a `duplicate_of`.
    #[serde(default)]
    pub status: Option<LifecycleStatusV1>,
    #[serde(default)]
    pub pr: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub duplicate_of: Option<String>,
    /// `set_lifecycle`: shown in the history.
    #[serde(default)]
    pub note: Option<String>,
    /// `set_criterion`. Refused once a criterion exists and a run started or
    /// a pair was attached: a criterion chosen after the results proves
    /// nothing.
    #[serde(default)]
    pub criterion: Option<CriterionInputV1>,
    /// `set_criterion`: the scenario to measure; defaults to the one the
    /// suggestion's plan names.
    #[serde(default)]
    pub scenario_id: Option<String>,
    /// `set_verdict`. `validated_improvement` is refused unless a criterion
    /// was registered before the first attached pair or run, and the evidence
    /// has `min_runs` completed runs on both sides.
    #[serde(default)]
    pub outcome: Option<ValidationOutcomeV1>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub controls_checked: Vec<String>,
    /// The author when the caller has no identity.
    #[serde(default)]
    pub by: Option<String>,
    /// Stamped by the engine on every invocation; callers omit it.
    #[serde(rename = "_caller_worker_id", default)]
    pub caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ReviewsRequestV1 {
    /// Only this analysis; absent means every analysis.
    #[serde(default)]
    pub evaluation_id: Option<String>,
}

/// Suggestions of one analysis by lifecycle status; a suggestion nobody acted
/// on counts as `new`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewSummaryV1 {
    pub evaluation_id: String,
    pub suggestions: u32,
    pub new: u32,
    pub accepted: u32,
    pub in_progress: u32,
    pub shipped: u32,
    pub rejected: u32,
    pub duplicate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewsResponseV1 {
    /// The stored rows: suggestions somebody acted on or validated.
    pub reviews: Vec<SuggestionReviewV1>,
    /// One per analysis that has suggestions, newest first.
    pub summaries: Vec<ReviewSummaryV1>,
}

/// `eval::start-validation`: builds a baseline and a candidate stack from the
/// E2E's `harness` template pinned to two pushed commits and starts both
/// executions in Docker. Spends model tokens, so it only ever runs on an
/// explicit request. With `dry_run` it only resolves and checks the refs.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StartValidationRequestV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
    /// The harness-e2e scenario to run.
    pub scenario_id: String,
    /// A ref of `code_repository` (a branch, a tag or a commit) that is on a
    /// remote branch.
    pub candidate_ref: String,
    /// Defaults to the merge base of the candidate and `origin/main`.
    #[serde(default)]
    pub baseline_ref: Option<String>,
    /// Runs of the scenario for each side; defaults to 5 (1-20).
    #[serde(default)]
    pub runs: Option<u32>,
    /// Required unless `dry_run`.
    #[serde(default)]
    pub model: String,
    /// Required unless `dry_run`.
    #[serde(default)]
    pub provider: String,
    /// Registered first, before any execution starts. Required unless
    /// `dry_run`, which only checks it against `runs` when present.
    #[serde(default)]
    pub criterion: Option<CriterionInputV1>,
    /// The author: this name wins over the host's user and the caller's
    /// identity.
    #[serde(default)]
    pub by: Option<String>,
    /// Resolve and check the refs, the scenario and the runs exactly as a
    /// start does, and answer with the commits they resolve to. Registers
    /// nothing, calls no E2E function and spends nothing.
    #[serde(default)]
    pub dry_run: bool,
    /// Stamped by the engine on every invocation; callers omit it.
    #[serde(rename = "_caller_worker_id", default)]
    pub caller_worker_id: Option<String>,
}

/// One side of a validation, resolved to the commit the E2E would build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCommitV1 {
    /// The full commit.
    pub commit: String,
    /// The first 12 characters of `commit`.
    pub short: String,
    /// A remote branch that holds the commit, as `git branch -r` names it
    /// (`origin/feat`); the one named like the ref when several do.
    pub branch: String,
}

/// What `eval::start-validation` answers to a `dry_run`: the commits both
/// refs resolve to, every start check passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationResolutionV1 {
    pub baseline: ResolvedCommitV1,
    pub candidate: ResolvedCommitV1,
    /// What a start would go ahead with but a person should know.
    pub warnings: Vec<String>,
}

/// The suggestion's row once the executions started, or, for a `dry_run`, the
/// resolution.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum StartValidationResponseV1 {
    Started(Box<SuggestionReviewV1>),
    Resolved(ValidationResolutionV1),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RecurrenceRequestV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
}

/// One pattern over a set of analyses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecurrencePatternV1 {
    pub pattern: String,
    /// Occurrences summed over the analyses.
    pub occurrences: u32,
    /// Analyses with at least one occurrence.
    pub analyses_with: u32,
    /// Occurrences per analysis; absent when there is no analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_analysis: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecurrenceWindowV1 {
    /// Analyses whose collection finished and whose Harness version is known.
    pub analyses: u32,
    pub patterns: Vec<RecurrencePatternV1>,
}

/// Whether the suggestion's patterns came back: the analyses on Harness
/// versions before the one that shipped it against the ones from it on.
/// Analyses are kept 30 days, so the earlier window is bounded by retention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecurrenceResponseV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
    /// The version the suggestion shipped in, where the windows split.
    pub version: String,
    pub before: RecurrenceWindowV1,
    pub from_version: RecurrenceWindowV1,
    /// Analyses left out because they carry no (or no semantic) Harness
    /// version.
    pub without_version: u32,
}

// ---------------------------------------------------------------------------
// Reproduction at the decision point
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

/// The observed turn's options as the Harness stored them in its turn record,
/// which keeps only a session's latest turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnCaptureV1 {
    pub turn_id: String,
    /// The Harness's `TurnOptions`, unmodified.
    pub options: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_surface_digest: Option<String>,
    /// Tokens `pre_generate` hooks added at the turn's last step: above zero,
    /// the system prompt the model saw is not stored anywhere.
    #[serde(default)]
    pub hook_guidance_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReproductionStateV1 {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReproductionChangeKindV1 {
    /// The context as the model saw it.
    None,
    /// The suggestion's own `check.change`.
    Proposed,
    /// Edits a person wrote.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FidelityLevelV1 {
    /// The provider counts the rebuilt request like the original, up to a
    /// fixed per-request overhead measured at the turn's first step.
    Exact,
    Approximate,
}

/// How faithfully the request at the decision point was rebuilt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FidelityV1 {
    pub level: FidelityLevelV1,
    /// Input tokens of the original step: input + cache read + cache write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counted_tokens: Option<u64>,
    /// `provider` when the provider counted, else the estimator's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator: Option<String>,
    /// The unexplained difference over the recorded tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_ratio: Option<f64>,
    /// Why the rebuild is approximate; empty when exact.
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// One function call of a reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyCallV1 {
    /// The function called (through `agent_trigger` or directly).
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The payload as JSON text, cut at 600 characters.
    pub payload: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyUsageV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// One reply at the decision point: a sample, or the original step read the
/// same way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyV1 {
    pub index: u32,
    /// Whether the signal shows in this reply; null when Jev answered
    /// unclear, the reply failed or it was not classified yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<bool>,
    pub calls: Vec<ReplyCallV1>,
    /// The opening of the reply's reasoning and of its text, 400 characters each.
    pub thinking: String,
    pub text: String,
    #[serde(default)]
    pub usage: ReplyUsageV1,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A replay of the decision point: N replies sampled from the rebuilt
/// request, with or without a change. No function ever runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReproductionV1 {
    pub id: String,
    pub change_kind: ReproductionChangeKindV1,
    #[serde(default)]
    pub change: Vec<ChangeEditV1>,
    /// The decision point and signal this replay used.
    pub check: SuggestionCheckV1,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Replies asked for, extensions included.
    pub requested: u32,
    pub state: ReproductionStateV1,
    /// `sampling` or `classifying` while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default)]
    pub samples: Vec<ReplyV1>,
    /// The original reply at the decision point, read the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<ReplyV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fidelity: Option<FidelityV1>,
    /// Known cost of the samples; null while no reply reported one. Only a
    /// lower bound while `cost_unknown_samples` is above zero. Jev's calls are
    /// in tokens below, never priced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Samples that answered without reporting a cost: unknown, not in
    /// `cost_usd`, and never estimated.
    #[serde(default)]
    pub cost_unknown_samples: u32,
    #[serde(default)]
    pub judge_input_tokens: u64,
    #[serde(default)]
    pub judge_output_tokens: u64,
    pub by: String,
    pub started_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a reproduction changes in the request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReproduceChangeV1 {
    #[default]
    None,
    Proposed,
    Custom {
        edits: Vec<ChangeEditV1>,
    },
}

/// `eval::reproduce`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReproduceRequestV1 {
    pub evaluation_id: String,
    pub suggestion_index: usize,
    #[serde(default)]
    pub change: ReproduceChangeV1,
    /// Replies to sample, 1–50; default 20.
    #[serde(default)]
    pub samples: Option<u32>,
    /// Add replies to this earlier reproduction of the suggestion, with its
    /// change; with `samples: 0`, only finish what a failure left undone.
    #[serde(default)]
    pub extend: Option<String>,
    /// The decision point and signal, for a suggestion without a `check` (an
    /// older analysis) or to override it.
    #[serde(default)]
    pub check: Option<SuggestionCheckV1>,
    /// Rebuild the request and check its fidelity without sampling: nothing
    /// is spent or stored.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub by: Option<String>,
    /// Stamped by the engine on every invocation; callers omit it.
    #[serde(rename = "_caller_worker_id", default)]
    pub caller_worker_id: Option<String>,
}

/// What a dry run reports before anything is spent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReproducePreviewV1 {
    pub fidelity: FidelityV1,
    pub original: ReplyV1,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// What the original step cost; null when the provider reported nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_cost_usd: Option<f64>,
    pub samples: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReproduceResponseV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<ReproducePreviewV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<SuggestionReviewV1>,
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
