//! The one public contract of the Harness E2E improvement loop: its
//! configuration, the cycle record both inputs (an objective or an E2E
//! execution) converge on, the durable evidence and the decision.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::candidate::{
    CandidateConfigV1, CandidateIdentityV1, CandidatePublishV1, CandidateV1, ProposalIdentityV1,
    ProposalV1,
};
use crate::error::EvalError;
use crate::pull_request::PullRequestV1;

/// Identity of the detectors, first pass and coverage rules; part of every
/// `policy_revision`.
pub const RULES_VERSION: &str = "e2e-loop-rules/5";
pub const CYCLE_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_LIST_LIMIT: u32 = 50;
pub const MAX_LIST_LIMIT: u32 = 200;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Where the E2E runs a dispatched execution (`ExecutionParameters.where`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WhereV1 {
    /// The E2E's own Harness: observes the current stack, cannot pin a
    /// candidate commit.
    Harness,
    Docker,
    Github,
}

/// The model the E2E subject runs on. Credentials stay in the providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectV1 {
    pub provider: String,
    pub model: String,
    /// Agent profile the subject runs under; absent uses the E2E default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// A stack in the E2E's own shape (`ExecutionStack`), e.g. `package://harness`
/// pinned to `commit:<40 hex>` of `iii-hq/workers` path `harness`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StackV1 {
    pub name: String,
    pub yaml: String,
}

/// The minimum effect a plan freezes for each metric it can target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MinEffectV1 {
    /// Total tokens per run.
    pub tokens: f64,
    /// Percentage points of pass rate.
    pub pass_rate_pp: f64,
}

/// The limits of one cycle, covering every kind of its work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleBudgetV1 {
    pub max_spend_usd: f64,
    /// Wall time since the cycle was created.
    pub max_duration_ms: i64,
    /// Dispatched work items, retries included.
    pub max_calls: u32,
    /// Candidates the cycle may try.
    pub max_candidates: u32,
}

/// Everything a cycle needs to spend on the E2E. Nothing has a default: an
/// unconfigured loop selects coverage but dispatches nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopConfigV1 {
    pub subject: SubjectV1,
    #[serde(rename = "where")]
    pub where_: WhereV1,
    /// Required for `docker` and `github`; refused for `harness`, which runs
    /// on its own stack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<StackV1>,
    pub runs_per_side: u32,
    pub technical_retries: u8,
    /// Scenarios one plan may run at most.
    pub max_scenarios: u32,
    /// What one run is reserved at before dispatch; an unreported actual
    /// cost stays charged at it, never at zero.
    pub estimated_usd_per_run: f64,
    pub min_effect: MinEffectV1,
    pub budget: CycleBudgetV1,
    /// Control scenarios every plan also runs and preserves (pass rate and
    /// every spec criterion), e.g. `minimal_path`.
    #[serde(default)]
    pub controls: Vec<String>,
    /// Semantic triage of ambiguous findings through `judge::evaluate`;
    /// absent: an ambiguous finding stays inconclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<JudgeConfigV1>,
    /// The bounded read-only proposal and the isolated candidate actor;
    /// absent: a cycle stops before any proposal, inconclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateConfigV1>,
    /// The stack that builds the workers under test from a commit; required
    /// to validate a candidate. With it, the baseline runs the candidate's
    /// base commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationConfigV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JudgeConfigV1 {
    /// The judge provider (`judge::evaluate` hub routing), e.g. `typesafe`.
    pub provider: String,
    /// What one grouped question is reserved at.
    pub estimated_usd: f64,
}

/// The E2E stack in the E2E's own shape. Every worker a candidate may change
/// is declared as a `package://<worker>` container with `commit: {commit}`,
/// so both sides build it from their own commit; everything else (Engine,
/// evaluator, unchanged workers) stays fixed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidationConfigV1 {
    #[serde(rename = "where")]
    pub where_: WhereV1,
    pub stack_name: String,
    /// Stack YAML pinning a source revision; every `{commit}` is replaced by
    /// the 40-hex sha.
    pub source_stack_template: String,
}

impl ValidationConfigV1 {
    pub fn source_stack(&self, commit: &str) -> StackV1 {
        StackV1 {
            name: self.stack_name.clone(),
            yaml: self.source_stack_template.replace("{commit}", commit),
        }
    }

    /// The workers the source stack builds from the commit: each container
    /// `package://<worker>` whose `commit` is `{commit}`.
    pub fn built_from_commit(&self) -> std::collections::BTreeSet<String> {
        // A letters-only stand-in stays a YAML string however it is quoted.
        const PIN: &str = "candidatecommit";
        let yaml = self.source_stack_template.replace("{commit}", PIN);
        let Ok(stack) = serde_yaml::from_str::<serde_yaml::Value>(&yaml) else {
            return Default::default();
        };
        stack["containers"]
            .as_mapping()
            .into_iter()
            .flatten()
            .filter(|(_, container)| container["commit"].as_str() == Some(PIN))
            .filter_map(|(_, container)| container["worker"].as_str()?.strip_prefix("package://"))
            .map(str::to_string)
            .collect()
    }
}

impl LoopConfigV1 {
    pub fn validate(&self) -> Result<(), EvalError> {
        let invalid = |reason: &str| Err(EvalError::InvalidRequest(reason.into()));
        let positive = |value: f64| value.is_finite() && value > 0.0;
        if self.subject.provider.trim().is_empty() || self.subject.model.trim().is_empty() {
            return invalid("subject.provider and subject.model must name a model");
        }
        match (self.where_, &self.stack) {
            (WhereV1::Harness, Some(_)) => {
                return invalid("where harness runs on its own stack: omit stack")
            }
            (WhereV1::Docker | WhereV1::Github, None) => {
                return invalid("where docker or github needs a stack {name, yaml}")
            }
            (_, Some(stack)) if stack.name.trim().is_empty() || stack.yaml.trim().is_empty() => {
                return invalid("stack.name and stack.yaml must not be empty")
            }
            _ => {}
        }
        if self.runs_per_side == 0 || self.max_scenarios == 0 {
            return invalid("runs_per_side and max_scenarios must be at least 1");
        }
        if !positive(self.estimated_usd_per_run) {
            return invalid("estimated_usd_per_run must be a positive number of US dollars");
        }
        if !positive(self.min_effect.tokens) || !positive(self.min_effect.pass_rate_pp) {
            return invalid("min_effect.tokens and min_effect.pass_rate_pp must be positive");
        }
        let budget = &self.budget;
        if !positive(budget.max_spend_usd)
            || budget.max_duration_ms <= 0
            || budget.max_calls == 0
            || budget.max_candidates == 0
        {
            return invalid("every budget limit must be positive");
        }
        if let Some(judge) = &self.judge {
            if judge.provider.trim().is_empty()
                || !judge.estimated_usd.is_finite()
                || judge.estimated_usd < 0.0
            {
                return invalid("judge.provider must be set and judge.estimated_usd be 0 or more");
            }
        }
        if let Some(candidate) = &self.candidate {
            // Checked here, not first at the proposal after a paid baseline.
            crate::candidate::validate_config(candidate)
                .map_err(|error| EvalError::InvalidRequest(error.to_string()))?;
            // The sign test needs five same-direction pairs: with fewer,
            // 0.5^n never reaches alpha 0.05 and no candidate can be accepted.
            if self.runs_per_side < 5 {
                return invalid("candidate work needs runs_per_side of at least 5");
            }
        }
        if let Some(validation) = &self.validation {
            if self.candidate.is_none() {
                return invalid("validation needs candidate: only a candidate is validated");
            }
            if validation.where_ == WhereV1::Harness {
                return invalid("validation pins the Harness: where must be docker or github");
            }
            if !validation.source_stack_template.contains("{commit}")
                || validation.stack_name.trim().is_empty()
            {
                return invalid("validation needs a stack_name and a template with {commit}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigureRequestV1 {
    pub config: LoopConfigV1,
    /// Stamped by the engine on every invocation; callers omit it.
    #[serde(rename = "_caller_worker_id", default)]
    pub caller_worker_id: Option<String>,
}

/// The stored configuration. `revision` (SHA-256 of the config and the
/// rules) is the `policy_revision` every plan freezes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredConfigV1 {
    pub config: LoopConfigV1,
    pub revision: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ConfigRequestV1 {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigResponseV1 {
    /// `null` until `eval::configure`.
    pub config: Option<StoredConfigV1>,
    pub rules_version: String,
}

// ---------------------------------------------------------------------------
// The cycle
// ---------------------------------------------------------------------------

/// What started a cycle: `{"objective": "..."}` or `{"execution_id": "..."}`.
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
    /// Durably paused on an external event: an E2E execution to finish, or
    /// the candidate stage (`next`).
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

impl CycleStatusV1 {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Where the cycle is. An objective goes coverage, baseline, analysis,
/// proposal, candidate, validation, analysis, pull request; an execution starts
/// at analysis and, with a pertinent finding, goes coverage, triage (paid only when
/// covered and ambiguous), then baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CycleStageV1 {
    Coverage,
    Baseline,
    AwaitingExecution,
    Analysis,
    /// Ambiguous findings judged once (`judge::evaluate`), or the recorded
    /// reason no judgment is needed.
    Triage,
    /// The bounded read-only proposal turning evidence into a brief.
    Proposal,
    /// The isolated candidate actor.
    Candidate,
    /// The candidate's source validation execution.
    Validation,
    /// The accepted candidate's pull request, ready for review.
    PullRequest,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleEventV1 {
    pub at: i64,
    pub status: CycleStatusV1,
    pub stage: CycleStageV1,
    pub detail: String,
}

/// Why an execution belongs to the cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRoleV1 {
    /// The execution that started the cycle.
    Source,
    Baseline,
    /// The candidate's source validation.
    Candidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStateV1 {
    /// Ownership recorded, `execution-start` called or about to be: its id
    /// may be unknown; it is reconciled by `work_id`, never dispatched again.
    Dispatching,
    Running,
    /// Terminal and its reports copied into the loop's evidence.
    Imported,
    /// The dispatch was refused, or never appeared in the E2E.
    Lost,
}

/// One E2E execution the cycle owns. Recorded before dispatch; `work_id` is
/// sent to the E2E as its immutable idempotency key. `label` is display only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleExecutionV1 {
    pub role: ExecutionRoleV1,
    pub state: ExecutionStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The budget reservation it was dispatched under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One criterion of a test spec (`spec.criteria[]`), the id run reports
/// award points under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestCriterionV1 {
    pub id: String,
    pub description: String,
    pub gate: bool,
}

/// A test selected as coverage hypothesis, with the executable identity of
/// its CURRENT definition from `e2e::scenarios-list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectedTestV1 {
    pub scenario_id: String,
    pub case_id: String,
    pub seed: u64,
    pub behavior_sha256: String,
    pub contract_sha256: String,
    pub inputs_sha256: String,
    pub criteria: Vec<TestCriterionV1>,
    /// The deliverable invariants the catalog declares.
    pub invariants: Vec<String>,
    /// Spec words that matched each required term group.
    pub matched_terms: Vec<String>,
    pub rationale: String,
    /// Set when the catalog row's history is of another version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_note: Option<String>,
}

/// The objective mapped onto the E2E catalog, before any spend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoverageV1 {
    /// The need the objective maps to (`context_consumption`,
    /// `provider_fallback`, `skills`), or `unmapped`.
    pub need: String,
    pub covered: bool,
    pub selected: Vec<SelectedTestV1>,
    /// The configured controls, each in its current definition.
    #[serde(default)]
    pub controls: Vec<SelectedTestV1>,
    /// The exact missing coverage when `covered` is false; limitations else.
    pub gaps: Vec<String>,
    pub tests_total: u32,
    pub tests_revision: String,
    pub catalog_sha256: String,
    pub selected_at: i64,
}

/// The deterministic first pass over one imported execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleAnalysisV1 {
    pub execution_id: String,
    pub admission_key: String,
    pub role: ExecutionRoleV1,
    pub identity: EvidenceIdentityV1,
    pub findings: Vec<FindingV1>,
    /// What the stored copy could not tell; never assumed.
    pub unknowns: Vec<String>,
    /// True only when the report says every child transcript was kept.
    pub child_transcripts_complete: bool,
    /// What `sentinel::reports::ingest` answered for the runs' recorded
    /// failures; absent when the reports recorded none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentinel: Option<SentinelForwardV1>,
    /// What each reported scenario expects and how many runs it had, for
    /// the judge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<ScenarioContextV1>,
    pub analyzed_at: i64,
}

/// One reported scenario as the E2E describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioContextV1 {
    pub scenario_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub runs: u32,
}

/// The runs' recorded failures forwarded to the central Sentinel. Its
/// classification and triage are kept as it answered and reused, never
/// judged again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SentinelForwardV1 {
    pub observations: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduped: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    /// `results[]` as Sentinel answered: dedupe_key, classification,
    /// outcome, group_id, occurrence_id, group_status, triage.
    #[serde(default)]
    pub results: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The baseline measured against the frozen plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaselineV1 {
    pub execution_id: String,
    pub primary: EvidenceSideV1,
    /// In the plan's `preserved` order.
    pub preserved: Vec<EvidenceSideV1>,
    /// Every criterion has its runs and a value: a candidate can be compared.
    pub usable: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NextActionV1 {
    AwaitExecution,
    /// Evidence-backed candidate work, coordinated by a later stage.
    CandidateWork,
}

/// The durable next step of a waiting cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleNextV1 {
    pub action: NextActionV1,
    pub detail: String,
    /// Fingerprints of the findings it rests on.
    #[serde(default)]
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeCodeV1 {
    /// The objective has no E2E coverage; `coverage.gaps` names it.
    NoCoverage,
    /// The analysis found nothing worth candidate work.
    NoAction,
    /// A limit of the cycle was reached.
    Budget,
    /// The evidence cannot support a conclusion (e.g. an unusable baseline).
    Inconclusive,
    /// `decision` holds accepted, rejected or inconclusive, and no pull
    /// request was opened.
    Decided,
    /// The accepted change is in a pull request ready for review
    /// (`pull_request`). Nothing is merged or released by the loop.
    Proposed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleOutcomeV1 {
    pub code: OutcomeCodeV1,
    pub detail: String,
}

/// The one record of a cycle, whichever input started it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleRecordV1 {
    pub schema_version: u32,
    pub cycle_id: String,
    pub input: CycleInputV1,
    pub status: CycleStatusV1,
    pub stage: CycleStageV1,
    /// Durable step counter; a queued step that does not match is stale.
    pub step: u64,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    /// Oldest first.
    pub history: Vec<CycleEventV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<CoverageV1>,
    /// Frozen before any dispatch; never changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<CyclePlanV1>,
    #[serde(default)]
    pub executions: Vec<CycleExecutionV1>,
    #[serde(default)]
    pub analyses: Vec<CycleAnalysisV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<CycleNextV1>,
    /// Recorded once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<CycleDecisionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CycleOutcomeV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// The configuration frozen when the cycle was created: later edits of
    /// the global configuration never change its actor, budget, target or
    /// criteria. `null`: created unconfigured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<StoredConfigV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triage: Option<TriageV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ProposalStateV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateStateV1>,
    /// The accepted candidate's pull request, once opened or found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequestV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriageSourceV1 {
    /// An explicit objective: no authorization needed.
    Objective,
    /// A clear deterministic defect.
    Deterministic,
    /// Sentinel's triage of the same occurrences, reused.
    Sentinel,
    Jev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TriageOutcomeV1 {
    Proceed,
    NoAction,
    /// The semantic step is unknown (unavailable, invalid or lost answer).
    Unknown,
}

/// Why the cycle proceeds past its findings, or stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriageV1 {
    pub source: TriageSourceV1,
    pub outcome: TriageOutcomeV1,
    pub findings: Vec<String>,
    pub reason: String,
    /// Set before `judge::evaluate` is called and sent again after a
    /// restart or an unavailable judge: the question has no side effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    /// Sentinel triage of the same occurrences, as Sentinel answered.
    #[serde(default)]
    pub reused: Vec<Value>,
}

/// The proposal's identity (persisted before it starts) and latest state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposalStateV1 {
    pub identity: ProposalIdentityV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<ProposalV1>,
}

/// The candidate's identity (persisted before it starts), latest state,
/// tree and the branch it was published to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CandidateStateV1 {
    pub identity: CandidateIdentityV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_tree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<CandidatePublishV1>,
}

/// The E2E `ExecutionParameters` the plan dispatches, frozen with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionParametersV1 {
    pub provider: String,
    pub model: String,
    pub runs: u32,
    pub scenarios: Vec<String>,
    pub technical_retries: u8,
    #[serde(rename = "where")]
    pub where_: WhereV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<StackV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// How many measured runs each side needs and the cases they run on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SamplingPlanV1 {
    pub runs_per_side: u32,
    /// The canonical `case_id`s both sides must report.
    #[serde(default)]
    pub seeds: Vec<String>,
}

/// What a cycle commits to before any result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CyclePlanV1 {
    pub objective: String,
    /// The selected tests and controls, each with the executable identity
    /// of its current definition, frozen with the plan.
    pub tests: Vec<SelectedTestV1>,
    /// The metric, its direction, the minimum effect and the scenario.
    pub criterion: CriterionV1,
    /// Correctness the candidate must keep: a loss rejects, an unmeasured one
    /// is inconclusive.
    #[serde(default)]
    pub preserved: Vec<CriterionV1>,
    #[serde(default)]
    pub controls: Vec<String>,
    pub sampling: SamplingPlanV1,
    pub parameters: ExecutionParametersV1,
    /// What the baseline is reserved at: runs × scenarios × the configured
    /// per-run estimate.
    pub estimated_usd: f64,
    /// The configuration revision the cycle runs under.
    pub policy_revision: String,
    pub frozen_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CycleWorkKindV1 {
    Investigation,
    /// The candidate actor.
    Implementation,
    Jev,
    E2e,
    Control,
    Retry,
}

/// One piece of work a cycle is about to dispatch. `work_id` is stable
/// across redelivery and restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleWorkV1 {
    pub work_id: String,
    pub kind: CycleWorkKindV1,
    pub estimated_usd: f64,
    #[serde(default)]
    pub candidate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkReservationV1 {
    pub work: CycleWorkV1,
    pub reserved_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<i64>,
    /// The reported cost; `None` once settled means unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// A cycle's budget and reservations, written only with compare-and-set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleLedgerV1 {
    pub cycle_id: String,
    pub budget: CycleBudgetV1,
    pub started_at: i64,
    #[serde(default)]
    pub reservations: BTreeMap<String, WorkReservationV1>,
}

/// What a cycle has drawn on its budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleUsageV1 {
    pub budget: CycleBudgetV1,
    /// Reported costs plus the estimate of work without one.
    pub spend_usd: f64,
    /// Work settled without a reported cost.
    pub unknown_cost: u32,
    pub calls: u32,
    pub candidates: u32,
    pub elapsed_ms: i64,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Exactly one of `objective` or `execution_id`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct CycleStartRequestV1 {
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub execution_id: Option<String>,
}

impl CycleStartRequestV1 {
    pub fn input(&self) -> Result<CycleInputV1, EvalError> {
        let clean = |text: &Option<String>| {
            text.as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        match (clean(&self.objective), clean(&self.execution_id)) {
            (Some(objective), None) => Ok(CycleInputV1::Objective(objective)),
            (None, Some(execution_id)) => Ok(CycleInputV1::ExecutionId(execution_id)),
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleListResponseV1 {
    pub cycles: Vec<CycleRecordV1>,
    /// What each listed cycle drew on its budget, by `cycle_id`; a cycle
    /// that reserved no work is absent.
    #[serde(default)]
    pub usage: BTreeMap<String, CycleUsageV1>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ExecutionsRequestV1 {
    /// Passed to the E2E's own execution search.
    #[serde(default)]
    pub query: Option<String>,
}

/// Recent finished E2E executions a cycle can start from, newest first.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionsResponseV1 {
    pub executions: Vec<ExecutionOptionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOptionV1 {
    pub execution_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// The cycle that already imported it; a second cycle would stop at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
    /// Empty when it is in a cycle or its reports could not be read.
    pub scenarios: Vec<ExecutionScenarioV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionScenarioV1 {
    pub scenario_id: String,
    /// The E2E's mean score, 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_score: Option<f64>,
    /// What a cycle from this execution would restore; absent: nothing
    /// measurable was lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleResultResponseV1 {
    pub cycle: CycleRecordV1,
    /// `null` until the cycle reserves work.
    pub usage: Option<CycleUsageV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleCancelResponseV1 {
    pub cancelled: bool,
    pub status: CycleStatusV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepRequestV1 {
    pub cycle_id: String,
    pub step: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepResponseV1 {
    pub skipped: bool,
    pub status: CycleStatusV1,
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
    pub reconciled: u64,
    pub admitted: u64,
    pub expired: u64,
}

/// An `e2e::dashboard::changed` event (`{kind, execution_id, at}`): only a
/// hint to re-read `execution-get`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ExecutionChangedEventV1 {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub execution_id: Option<String>,
    #[serde(default)]
    pub at: Option<String>,
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
// Admission and durable evidence
// ---------------------------------------------------------------------------

/// The claim of one execution/run/attempt, set once with compare-and-set:
/// redelivery and restart never admit twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionV1 {
    pub admission_key: String,
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    pub cycle_id: String,
    pub role: ExecutionRoleV1,
    pub claimed_at: i64,
}

/// Where the admission sweep resumes: executions started before `since`
/// were already looked at.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdmissionCheckpointV1 {
    pub since: i64,
    pub updated_at: i64,
}

/// The executable identity of one reported scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioIdentityV1 {
    pub scenario_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_sha256: Option<String>,
}

/// Identity of an imported execution as its reports state it. `None` is
/// unknown (absent, or reports that disagree), never assumed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceIdentityV1 {
    #[serde(default)]
    pub scenarios: Vec<ScenarioIdentityV1>,
    pub contract_digest: Option<String>,
    pub artifacts_digest: Option<String>,
    pub stack_digest: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub harness_version: Option<String>,
    /// SHA-256 of `/proc/<pid>/exe` of the ready Harness, captured by the
    /// E2E runner (`system_under_test.harness_binary_sha256`), never from the
    /// subject. Known only when every report agrees. Like the Harness
    /// version it is the product side, kept per side and never part of the
    /// fixed comparability; it binds the run to the commit's build.
    #[serde(default)]
    pub harness_binary_sha256: Option<String>,
    pub workers_revision: Option<String>,
    pub engine_version: Option<String>,
    pub e2e_revision: Option<String>,
    /// Digest of the full native subject configuration (model, provider,
    /// agent profile) as the reports state it.
    #[serde(default)]
    pub subject_sha256: Option<String>,
    /// Digest of every stack component but the Harness, as the reports
    /// prove it; unknown when they cannot.
    #[serde(default)]
    pub fixed_stack_digest: Option<String>,
}

/// The native `execution-get` bundle copied into the loop at admission, so
/// analysis survives teardown and the E2E's retention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidenceV1 {
    pub admission_key: String,
    pub execution_id: String,
    pub imported_at: i64,
    pub bundle_digest: String,
    pub bundle: Value,
    pub identity: EvidenceIdentityV1,
}

/// The six classes the first pass sorts a finding into.
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
    /// Only these justify Harness candidate work. An evaluation gap is the
    /// evaluator's to close, never a reason to change the Harness.
    pub fn is_pertinent(self) -> bool {
        matches!(self, Self::Defect | Self::ImprovementOpportunity)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingV1 {
    pub fingerprint: String,
    pub class: FindingClassV1,
    /// The detector rule or execution signal it comes from.
    pub signal: String,
    pub observation: String,
    /// What the lost E2E criterion asks (its description), for the judge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asks: Option<String>,
}

// ---------------------------------------------------------------------------
// Deterministic diagnostics over run transcripts
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticV1 {
    pub rule_id: String,
    pub rule_version: String,
    pub fingerprint: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub target: String,
    pub observation: String,
    pub correlation: CorrelationV1,
    pub evidence: Vec<EntryRefV1>,
}

impl DiagnosticV1 {
    /// `<rule_id>:<target>`, the recurring pattern.
    pub fn pattern(&self) -> String {
        format!("{}:{}", self.rule_id, self.target)
    }
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

// ---------------------------------------------------------------------------
// Comparison and decision
// ---------------------------------------------------------------------------

/// What a criterion measures, per measurable run, averaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CriterionMetricV1 {
    /// Occurrences of `pattern` per run, by the detectors.
    SignalPerRun,
    /// Share of runs that completed and passed (`task_incomplete` fails).
    PassRate,
    /// `awarded / possible` of the E2E criterion `criterion_id`; a run that
    /// did not evaluate it measures nothing.
    CriterionScore,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion_id: Option<String>,
    pub direction: DirectionV1,
    /// In the metric's unit (pp for pass rate and criterion score, s for
    /// duration); 0 makes any loss a regression.
    pub min_effect: f64,
    pub min_runs: u32,
    pub scenario_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOutcomeV1 {
    ValidatedImprovement,
    NoImprovement,
    Regression,
    Inconclusive,
}

/// One run as the E2E reported it; unreported values stay absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRunV1 {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_id: Option<String>,
    /// `completion == completed` with a `valid` technical outcome.
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<String>,
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
    /// `awarded / possible` per evaluated criterion id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub criteria: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals: Option<BTreeMap<String, u32>>,
}

impl EvidenceRunV1 {
    /// A run says something about the Harness only with a valid technical
    /// outcome.
    pub fn measurable(&self) -> bool {
        self.technical
            .as_deref()
            .is_some_and(|technical| technical.eq_ignore_ascii_case("valid"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceSideV1 {
    pub execution_id: String,
    pub runs: Vec<EvidenceRunV1>,
    pub n: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean: Option<f64>,
}

/// A criterion judged on a pair, in code.
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComparabilityV1 {
    pub comparable: bool,
    pub checks: Vec<ComparabilityCheckV1>,
}

/// One side of the compared pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionSideV1 {
    pub execution_id: String,
    pub identity: EvidenceIdentityV1,
    #[serde(default)]
    pub seeds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_revision: Option<String>,
    /// The source revision this side must have run, as the E2E recorded it
    /// (`workers_revision`); what an actor claims is not proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<String>,
    /// SHA-256 of the frozen stack template this side must have run: every
    /// stack component but the Harness pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_stack: Option<String>,
    /// SHA-256 of the stack YAML the E2E recorded for this side, with only
    /// its Harness pin put back to the template placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_stack: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcomeV1 {
    Accepted,
    Rejected,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CycleDecisionV1 {
    pub outcome: DecisionOutcomeV1,
    pub reason: String,
    pub policy_revision: String,
    pub baseline_execution_id: String,
    pub candidate_execution_id: String,
    pub comparability: ComparabilityV1,
    pub primary: EvidenceV1,
    pub preserved: Vec<EvidenceV1>,
    pub decided_at: i64,
}
