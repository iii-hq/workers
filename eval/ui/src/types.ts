export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }

export interface ContextSnapshot {
  session_id: string
  turn_id: string
  step: number
  model: string
  provider?: string
  estimator?: string
  usable: number
  effective_max_output_tokens: number
  total: number
  free: number
  categories: {
    system_prompt: number
    tools: number
    messages: {
      user: number
      assistant: number
      function_result: number
      custom: number
    }
    overhead: number
    hook_guidance: number
  }
  compacted: boolean
  summarized_head_tokens?: number
  usage?: JsonValue
  timestamp: number
}

export interface SessionMetrics {
  root_session_id: string
  complete: boolean
  totals: {
    sessions: number
    turns: number
    function_calls: number
    function_call_errors: number
    input_tokens?: number
    output_tokens?: number
    cache_read_tokens?: number
    cache_write_tokens?: number
    reasoning_tokens?: number
    cost_usd?: number
  }
  by_session: Array<{
    session_id: string
    parent_session_id?: string
    depth: number
    turns: number
    function_calls: number
    function_call_errors: number
    input_tokens?: number
    output_tokens?: number
    cache_read_tokens?: number
    cache_write_tokens?: number
    reasoning_tokens?: number
    cost_usd?: number
    context?: ContextSnapshot
  }>
  traces?: {
    trace_count: number
    span_count: number
    error_span_count: number
    duration_ms: number
    by_session: Array<{
      session_id: string
      parent_session_id?: string
      depth: number
      trace_count: number
      span_count: number
      error_span_count: number
      duration_ms: number
    }>
  }
}

export type TurnStatus = 'running' | 'awaiting_functions' | 'completed' | 'cancelled' | 'failed'

// Session monitor — mirrors eval/src/contract.rs (`eval::*` functions).

export type AnalysisStatus =
  | 'queued'
  | 'collecting'
  | 'judging'
  | 'investigating'
  | 'completed'
  | 'failed'
  | 'cancelled'

export type ThinkingLevel = 'minimal' | 'low' | 'medium' | 'high' | 'xhigh'

export interface MonitorModel {
  model: string
  provider: string
  thinking_level?: ThinkingLevel | null
  provider_options?: Record<string, JsonValue> | null
}

export interface MonitorConfig {
  enabled: boolean
  model: MonitorModel
  /** Absolute path of the codebase the investigation reads; absent means code access is off. */
  code_repository?: string
  /**
   * US dollars of known investigation cost per UTC day after which automatic observation admits nothing until the
   * next day; absent means no cap. A manual analysis is never refused by it.
   */
  daily_cost_cap_usd?: number
  revision: string
  updated_at: number
  enabled_since?: number
}

/** Why the latest turn was not admitted; rows saved before the cost cap existed read as `at_capacity`. */
export type RejectionReason = 'at_capacity' | 'cost_cap'

export interface CapacityRejection {
  session_id: string
  turn_id: string
  at: number
  reason: RejectionReason
}

/** Cost of the completed analyses that investigated with the configured model, provider and code access. */
export interface AnalysisCostStats {
  /** Analyses with a known cost; `min`, `median` and `max` come from them. */
  count: number
  min?: number
  median?: number
  max?: number
  /** Analyses that investigated but reported no cost: unknown, never zero. */
  unknown: number
}

/** The monitor's own spend, in dollars of investigation and replay LLM cost (Jev's usage is in tokens on each record). */
export interface MonitorCost {
  /** Start (ms since the Unix epoch) of the UTC day the `today_*` values cover. */
  since: number
  /** Everything known to be spent: `today_capture_usd` + `today_replay_usd`. The cap does not compare it. */
  today_usd: number
  /** What the analyses spent (the larger of the persisted spend and the sum of their `llm_cost_usd`): the only bucket the cap compares. */
  today_capture_usd: number
  /** What `eval::reproduce` samples spent: reported, never capped. */
  today_replay_usd: number
  /** Replay samples of the day that reported no cost: they add nothing to `today_replay_usd`, yet cost is unknown. */
  today_replay_unknown: number
  /** Analyses of the day whose investigation reported no cost: they add nothing to `today_capture_usd`, yet cost is unknown. */
  today_unknown: number
  cap_usd?: number
  /** A cap is set and `today_capture_usd` reached it: automatic observation admits nothing until the next UTC day. */
  capped: boolean
  per_analysis: AnalysisCostStats
}

export interface TriageAvailability {
  provider: string
  available: boolean
  code?: string
  models: string[]
  checked_at: number
}

/** What the monitor enforces, as `eval::config` reports it. The UI states these, never its own copy. */
export interface MonitorLimits {
  /** Deadline of one analysis, queue included. */
  analysis_budget_ms: number
  /** What one Jev call may take. */
  judge_timeout_ms: number
  /** Bytes of the evidence the models may read. */
  model_context_bytes: number
  assets_bytes: number
  investigation_max_turns: number
  investigation_max_output_tokens: number
  investigation_max_total_tokens: number
  /** Generate steps of an investigation that has code access. */
  investigation_code_max_turns: number
  /** Total tokens of an investigation that has code access. */
  investigation_code_max_total_tokens: number
  /** Analyses running at once. */
  queue_concurrency: number
  /** Unfinished analyses admitted (running and queued). */
  max_active_analyses: number
  retention_days: number
  retention_max_terminal: number
}

export interface MonitorState {
  config: MonitorConfig | null
  limits: MonitorLimits
  /** Requested, not engine-confirmed (SDK limitation). */
  observer_bound: boolean
  observer_error?: string
  last_rejection?: CapacityRejection
  triage?: TriageAvailability
  /** What the investigations cost so far, to decide before enabling. */
  cost: MonitorCost
}

export interface Failure {
  stage: AnalysisStatus
  code: string
  message: string
}

export type CoverageLevel = 'complete' | 'partial' | 'insufficient'

/**
 * Why the analyst was called. Only `needs_investigation` is produced now; the others are on analyses recorded before
 * a session was investigated on Jev's answer alone, and are kept so those still read.
 */
export type RoutingReason =
  | 'needs_investigation'
  | 'diagnostics'
  | 'insufficient_evidence'
  | 'low_confidence'
  | 'coverage_insufficient'
  | 'audit_sample'
  | 'manual_request'

export interface Routing {
  investigate: boolean
  reasons: RoutingReason[]
}

export interface JudgeCall {
  request_id: string
  started_at: number
  deadline: number
}

export interface AnalystRef {
  session_id: string
  turn_id?: string
  sent_at: number
}

export interface AnalysisCounters {
  sessions: number
  entries: number
  diagnostics: number
  suggestions: number
  rejected_suggestions: number
  validations: number
}

export interface StageTime {
  status: AnalysisStatus
  at: number
}

/** Known usage only: an absent field is unknown, never zero. */
export interface MonitorUsage {
  judge_calls: number
  judge_input_tokens: number
  judge_output_tokens: number
  judge_usage_complete: boolean
  llm_input_tokens?: number
  llm_output_tokens?: number
  llm_cost_usd?: number
}

export interface AnalysisRecord {
  schema_version: number
  evaluation_id: string
  observation_key: string
  origin: 'automatic' | 'manual'
  session_id: string
  turn_id: string
  source_title?: string
  model: MonitorModel
  /** The codebase directory, frozen at admission; absent means the investigation had no code access. */
  code_root?: string
  config_revision: string
  rules_version: string
  criteria_version: string
  status: AnalysisStatus
  step: number
  created_at: number
  updated_at: number
  deadline: number
  observe_since: number
  completed_at?: number
  counters: AnalysisCounters
  stages: StageTime[]
  usage: MonitorUsage
  coverage?: CoverageLevel
  routing?: Routing
  pending_reason?: string
  judge_call?: JudgeCall
  analyst?: AnalystRef
  failure?: Failure
  /** The earlier analysis of the same turn (same `observation_key`) this reanalysis was requested over. */
  supersedes?: string
  /** The Harness version the engine reported at admission (not necessarily the one that ran an older observed turn). */
  harness_version?: string
  /**
   * Occurrences of each deterministic pattern the collection found, keyed `<rule_id>:<target>` (a target may itself
   * contain colons). Absent when nothing was found and before collection (`coverage` is set once it read the evidence).
   */
  signals?: Record<string, number>
}

export interface EntryRef {
  session_id: string
  entry_id: string
}

export type Correlation = 'harness_notice_correlated' | 'unknown'

export interface Diagnostic {
  rule_id: string
  rule_version: string
  fingerprint: string
  session_id: string
  turn_id?: string
  target: string
  observation: string
  correlation: Correlation
  evidence: EntryRef[]
}

export interface ExcludedProbe {
  session_id: string
  turn_id?: string
  target: string
  code: string
  calls: string[]
}

export interface SessionEvidence {
  session_id: string
  parent_session_id?: string
  parent_turn_id?: string
  depth: number
  in_scope: boolean
  entries: number
  json_sha256?: string
  /** Masked, shortened entries shown to the models. */
  preview: JsonValue[]
  omitted_entries: number
  reduced_entries: number
}

export interface Coverage {
  level: CoverageLevel
  sessions_in_scope: number
  sessions_out_of_scope: number
  entries_read: number
  diagnostics_in_context: number
  context_bytes: number
  limitations: string[]
}

export interface Snapshot {
  captured_at: number
  rules_version: string
  source_session_id: string
  source_turn_id: string
  source_title?: string
  source_status: TurnStatus
  source_stop_reason?: string
  source_result_error?: string
  observed_model?: string
  observed_provider?: string
  /** The harness-e2e scenario the observed session ran, when it came from the E2E. */
  e2e_scenario?: string
  window_turn_ids: string[]
  sessions: SessionEvidence[]
  metrics_scope: 'session_tree'
  metrics: SessionMetrics
  diagnostics: Diagnostic[]
  excluded_probes: ExcludedProbe[]
  coverage: Coverage
}

export interface Stats {
  attempts: number
  requests: number
  questions: number
  input_tokens: number
  output_tokens: number
  elapsed_ms: number
  usage_complete: boolean
}

export interface ChoiceAnswer {
  type: 'choice'
  choice: string
  probabilities: Record<string, number>
  confidence: number
}

export type JudgeAnswer =
  | ChoiceAnswer
  | { type: 'noul'; noul: number }
  | {
      type: 'score'
      score: number
      probabilities: Record<string, number>
      confidence: number
      legend: Record<string, JsonValue>
    }

export interface Triage {
  provider: string
  request_id: string
  model: string
  criteria_version: string
  answers: Record<string, JudgeAnswer>
  stats: Stats
  completed_at: number
}

export interface TriageFailure {
  request_id: string
  code: string
  http_status?: number
  stats?: Stats
  message: string
}

export interface ValidationPlan {
  scenario_id: string | null
  reproduction: string
  invariants: string[]
  primary_metric: string
  expectation: string
  non_regression_controls: string[]
}

/** Lines of one file of the code directory the analyst read (1-based, inclusive). */
export interface CodeRef {
  /** Relative to the directory, never absolute. */
  path: string
  line_from: number
  line_to: number
}

export interface Suggestion {
  title: string
  observation: string
  hypothesis: string
  harness_component: string
  proposed_change: string
  expected_effect: string
  evidence: EntryRef[]
  /** At most 8, each checked against the directory; empty without code access. */
  code_refs: CodeRef[]
  limitations: string
  validation: ValidationPlan
  /** How to replay the step where the behavior happened; absent on older analyses. */
  check?: SuggestionCheck
}

export interface RejectedSuggestion {
  index: number
  title: string
  reasons: string[]
}

export type SignalVerdict = 'likely_expected' | 'worth_changing' | 'unclear'

export interface SignalAssessment {
  fingerprint: string
  verdict: SignalVerdict
  explanation: string
}

export interface Investigation {
  session_id: string
  turn_id: string
  requested_model: string
  requested_provider: string
  /** The directory this investigation ran in; absent without code access. */
  code_root?: string
  effective_model?: string
  effective_provider?: string
  suggestions: Suggestion[]
  rejected: RejectedSuggestion[]
  signal_assessments: SignalAssessment[]
  metrics?: SessionMetrics
  completed_at: number
}

export interface E2eReportRef {
  scenario_id: string
  subject_id?: string
  available: boolean
}

export interface E2eMeasure {
  average?: number
  /** Runs that reported it; below `run_count` means some reported nothing. */
  samples: number
}

export interface E2eScenario {
  scenario_id: string
  behavior_sha256?: string
  contract_fingerprint?: string
  run_count: number
  measures: Record<'function_calls' | 'function_call_errors' | 'tokens' | 'duration_seconds' | 'cost_usd', E2eMeasure>
  // The scenario's cohort in `plan_execution.measurements.cohorts[]`, present only when the execution has exactly
  // one for it. Each value is absent when the E2E did not report it, never zero.
  /** Share of the planned runs that passed (0 to 1). */
  pass_rate?: number
  /** Mean score of the scored runs (0 to 100). */
  mean_score?: number
  completed_runs?: number
  planned_runs?: number
  /** The cohort's total cost divided by its completed runs; the total also covers failed attempts. */
  cost_usd_per_run?: number
  /** The cohort's total tokens divided by its completed runs. */
  total_tokens_per_run?: number
  median_wall_time_ms?: number
  p50_function_calls?: number
}

export interface E2eExecution {
  execution_id: string
  label?: string
  status?: string
  conclusion?: string
  /** When the E2E started the execution (ms since the epoch): its results cannot predate it. */
  started_at?: number
  availability?: string
  reports_available: boolean
  reports: E2eReportRef[]
  evidence_error?: string
  model?: string
  provider?: string
  harness_version?: string
  engine_version?: string
  e2e_revision?: string
  scenarios: E2eScenario[]
  assessments?: { passed: number; total: number }
}

export interface ComparabilityCheck {
  field: string
  baseline?: string
  candidate?: string
  matches: boolean
}

export interface ValidationLink {
  suggestion_index: number
  baseline: E2eExecution
  candidate: E2eExecution
  comparability: { comparable: boolean; checks: ComparabilityCheck[] }
  attached_at: number
}

// Review: what people decide about a suggestion (`eval::review`, `eval::reviews`, `eval::start-validation`,
// `eval::recurrence`). Every row is kept in the `eval_suggestion` scope, apart from the analysis.

/** Where a suggestion stands. Only people move it: the monitor never does. */
export type LifecycleStatus = 'new' | 'accepted' | 'in_progress' | 'shipped' | 'rejected' | 'duplicate'

export interface LifecycleEvent {
  status: LifecycleStatus
  at: number
  by: string
  note?: string
}

export interface Lifecycle {
  status: LifecycleStatus
  /** The pull request that implements it (`in_progress`, `shipped`). */
  pr?: string
  /** The Harness version that shipped it, a semantic version like `1.8.43`; `eval::recurrence` splits the analyses at it. */
  version?: string
  /** Why it was rejected (`rejected`). */
  reason?: string
  /** What it duplicates: a pull request, or another `<evaluation_id>:<index>` (`duplicate`). */
  duplicate_of?: string
  /** Every change of status, oldest first; empty while `new`. */
  history: LifecycleEvent[]
}

/**
 * What a validation measures. Every metric but `signal_per_run` is the mean over the completed runs of the scenario
 * (`pass_rate` is the share that passed, `duration` is the wall time in milliseconds).
 */
export type CriterionMetric = 'signal_per_run' | 'pass_rate' | 'cost_usd' | 'duration' | 'tokens' | 'function_calls'

export type Direction = 'decrease' | 'increase'

/** What a person commits to before the results exist. */
export interface CriterionInput {
  metric: CriterionMetric
  /** `signal_per_run` only: one of the row's `patterns`. */
  pattern?: string
  /** The change that is wanted when the candidate moves the baseline mean in this direction. */
  direction: Direction
  /** Smallest relative change of the baseline mean that counts, as a fraction (`0.2` is 20%). Above 0; at most 1 for a decrease. */
  min_effect: number
  /** Completed runs each side needs for a verdict of improvement (1-20). */
  min_runs: number
}

export interface Criterion extends CriterionInput {
  /** The scenario whose runs are measured. */
  scenario_id: string
  registered_at: number
  registered_by: string
}

export type ValidationOutcome = 'validated_improvement' | 'no_improvement' | 'regression' | 'inconclusive'

export type ValidationRunState = 'starting' | 'running' | 'finished' | 'attached' | 'failed'

/** The baseline and candidate E2E executions this worker started for a suggestion (`eval::start-validation`). */
export interface ValidationRun {
  /** Absent until the E2E accepted the execution. */
  baseline_execution_id?: string
  candidate_execution_id?: string
  /** The full commits the Harness was built from. */
  baseline_commit: string
  candidate_commit: string
  scenario_id: string
  /** Runs requested for each side. */
  runs: number
  model: string
  provider: string
  started_at: number
  /** When both executions were seen ended; the sweep stops trying to attach the pair 30 minutes after. */
  finished_at?: number
  state: ValidationRunState
  /** Why it failed, or why attaching is being retried; starts with a stable code (`e2e_unavailable:`, `e2e_busy:`…). */
  error?: string
}

/** One run of the scenario as the E2E reported it. Runs the infrastructure broke are listed, never counted. */
export interface EvidenceRun {
  run_id: string
  /** The E2E reported the run complete with a valid technical outcome. */
  completed: boolean
  /** `completion` as the E2E reported it. */
  completion?: string
  /** `technical` as the E2E reported it; anything but `valid` is an infrastructure failure. */
  technical?: string
  /** `passed` or `failed`, the run's task outcome. */
  status?: string
  cost_usd?: number
  wall_time_ms?: number
  total_tokens?: number
  function_calls?: number
  /** Occurrences of each of the row's patterns the detectors found in the run's transcript (0 is a measurement); absent without a transcript. */
  signals?: Record<string, number>
}

export interface EvidenceSide {
  execution_id: string
  /** Every run of the scenario the execution holds. */
  runs: EvidenceRun[]
  /** Measurable runs (`technical` valid, finished or not) with a value of the criterion's metric (every one while there is none). */
  n: number
  /** Mean of the criterion's metric over those runs; absent without a criterion or without such runs. */
  mean?: number
}

/** Computed in code from the attached pair's runs, never by a model; the person's verdict is shown next to it. */
export interface Evidence {
  scenario_id: string
  computed_at: number
  baseline: EvidenceSide
  candidate: EvidenceSide
  computed_outcome: ValidationOutcome
  reason: string
}

/** The person's judgment, kept with the criterion it was made against. */
export interface Verdict {
  outcome: ValidationOutcome
  rationale: string
  /** The non-regression controls of the plan that were checked. */
  controls_checked: string[]
  by: string
  at: number
  criterion_snapshot?: Criterion
}

export interface SuggestionReview {
  evaluation_id: string
  suggestion_index: number
  /** Title, component and patterns are copied from the suggestion, so the row outlives the analysis's retention. */
  title: string
  harness_component: string
  /** The scenario the suggestion's validation plan names. */
  scenario_id?: string
  /** `<rule_id>:<target>` of the diagnostics the suggestion's evidence cites. */
  patterns: string[]
  lifecycle: Lifecycle
  criterion?: Criterion
  run?: ValidationRun
  /** When the first execution this worker started for the suggestion began, kept when a new start replaces `run`. */
  first_run_at?: number
  evidence?: Evidence
  verdict?: Verdict
  /** Replays of the decision point, oldest first. */
  reproductions?: Reproduction[]
}

/**
 * `eval::review` payload besides the analysis and suggestion. `by` is the author when the engine stamps no caller
 * identity; the api sends one.
 */
export type ReviewChange =
  | {
      action: 'set_lifecycle'
      /** Never back to `new`; a shipped suggestion stays shipped, to complete its `pr` or `version`. */
      status: Exclude<LifecycleStatus, 'new'>
      /** Required by `shipped` (here or already recorded). */
      pr?: string
      /** A semantic version, e.g. `1.8.43`. */
      version?: string
      /** Required by `rejected`. */
      reason?: string
      /** Required by `duplicate`. */
      duplicate_of?: string
      note?: string
      by?: string
    }
  | {
      action: 'set_criterion'
      criterion: CriterionInput
      /** Defaults to the scenario the suggestion's plan names. */
      scenario_id?: string
      by?: string
    }
  | {
      action: 'set_verdict'
      outcome: ValidationOutcome
      rationale: string
      controls_checked: string[]
      by?: string
    }

/** Suggestions of one analysis by lifecycle status; a suggestion nobody acted on counts as `new`. */
export interface ReviewSummary {
  evaluation_id: string
  suggestions: number
  new: number
  accepted: number
  in_progress: number
  shipped: number
  rejected: number
  duplicate: number
}

export interface ReviewsResponse {
  /** The stored rows: suggestions somebody acted on or validated. */
  reviews: SuggestionReview[]
  /** One per analysis that has suggestions, newest first. */
  summaries: ReviewSummary[]
}

/**
 * `eval::start-validation` payload besides the analysis and suggestion: builds a baseline and a candidate stack from
 * the E2E's `harness` template pinned to two pushed commits and starts both executions in Docker. Spends model tokens.
 */
export interface StartValidationParams {
  /** The harness-e2e scenario to run. */
  scenario_id: string
  /** A ref of `code_repository` (branch, tag or commit) that is on a remote branch. */
  candidate_ref: string
  /** Defaults to the merge base of the candidate and `origin/main`. */
  baseline_ref?: string
  /** Runs of the scenario for each side; defaults to 5 (1-20, at least `criterion.min_runs`). */
  runs?: number
  model: string
  provider: string
  /** Registered first, before any execution starts. */
  criterion: CriterionInput
  /** The author; without it the monitor credits the user it runs as. */
  by?: string
}

/** What the dry run of `eval::start-validation` needs: the refs, the scenario and the runs; no model, no criterion. */
export type ResolveValidationParams = Pick<StartValidationParams, 'scenario_id' | 'candidate_ref' | 'baseline_ref'> & {
  runs: number
}

/** One side of a validation resolved to the commit the E2E would build. */
export interface ResolvedCommit {
  /** The full commit. */
  commit: string
  /** Its first 12 characters. */
  short: string
  /** A remote branch that holds it, as `git branch -r` names it (`origin/feat`). */
  branch: string
}

/** The answer to a dry run: every start check passed. */
export interface ValidationResolution {
  baseline: ResolvedCommit
  candidate: ResolvedCommit
  /** What a start would go ahead with but a person should know. */
  warnings: string[]
}

/** One pattern over a set of analyses. */
export interface RecurrencePattern {
  pattern: string
  /** Occurrences summed over the analyses. */
  occurrences: number
  /** Analyses with at least one occurrence. */
  analyses_with: number
  /** Occurrences per analysis; absent when there is no analysis. */
  per_analysis?: number
}

export interface RecurrenceWindow {
  /** Analyses whose collection finished and whose Harness version is known. */
  analyses: number
  patterns: RecurrencePattern[]
}

/** Whether the suggestion's patterns came back: analyses on Harness versions before the shipped one against those from it on. */
export interface Recurrence {
  evaluation_id: string
  suggestion_index: number
  /** The version the suggestion shipped in, where the windows split. */
  version: string
  before: RecurrenceWindow
  from_version: RecurrenceWindow
  /** Analyses left out because they carry no (or no semantic) Harness version. */
  without_version: number
}

export interface AnalysisAssets {
  evaluation_id: string
  snapshot?: Snapshot
  triage?: Triage
  triage_failure?: TriageFailure
  investigation?: Investigation
  validations: ValidationLink[]
  /** The observed turn's options, copied when the evidence was captured. */
  capture?: { turn_id: string; hook_guidance_tokens: number }
}

export interface AnalysisResult {
  record: AnalysisRecord
  assets: AnalysisAssets
  /** One row per suggestion, in suggestion order: what people decided about it. A suggestion nobody acted on reads as `new`. */
  reviews: SuggestionReview[]
}

export interface CompletedEvent {
  evaluation_id: string
  status: AnalysisStatus
  timestamp: number
}

/** One `router::models::list` row. */
export interface CatalogModel {
  id: string
  provider: string
  display_name?: string
  context_window: number
  max_output_tokens: number
  supports_thinking?: boolean
  supports_xhigh?: boolean
  pricing?: ModelPricing
}

/** US dollars per million tokens; each price is absent when the provider does not publish it. */
export interface ModelPricing {
  input?: number
  output?: number
  cache_read?: number
  cache_write?: number
}

/** One scenario of the harness-e2e catalog (`e2e::dashboard::tests-list`); text may be empty. */
export interface E2eCatalogScenario {
  id: string
  title: string
  summary: string
}

// ---------------------------------------------------------------------------
// Reproduction at the decision point (`eval::reproduce`)
// ---------------------------------------------------------------------------

export type SignalRule = 'contract_rediscovery' | 'repeated_error_call'

/** How to recognize the behavior in one reply: exactly one of a rule computed in code or a yes/no question. */
export interface Signal {
  rule?: SignalRule
  question?: string
}

/** An edit of what the model saw: `target` is an entry id or `system_prompt`. */
export interface ChangeEdit {
  target: string
  find?: string
  replace?: string
  remove?: boolean
}

export interface SuggestionCheck {
  /** The assistant entry where the behavior happened. */
  decision_point: string
  signal: Signal
  /** The proposed change as edits of what the model saw; empty when it is not text the model reads. */
  change: ChangeEdit[]
}

export type ReproductionState = 'running' | 'completed' | 'failed'
export type ReproductionChangeKind = 'none' | 'proposed' | 'custom'

export interface Fidelity {
  level: 'exact' | 'approximate'
  recorded_tokens?: number
  counted_tokens?: number
  estimator?: string
  off_ratio?: number
  reasons: string[]
}

export interface ReplyCall {
  target: string
  description?: string
  /** JSON text, cut at 600 characters. */
  payload: string
}

export interface ReplyUsage {
  input_tokens?: number
  output_tokens?: number
  cache_read_tokens?: number
  cache_write_tokens?: number
  cost_usd?: number
}

/** One reply at the decision point: a sample, or the original step read the same way. */
export interface Reply {
  index: number
  /** Absent when unclear, failed or not classified yet. */
  signal?: boolean
  calls: ReplyCall[]
  thinking: string
  text: string
  usage: ReplyUsage
  duration_ms: number
  error?: string
}

export interface Reproduction {
  id: string
  change_kind: ReproductionChangeKind
  change: ChangeEdit[]
  check: SuggestionCheck
  model: string
  provider?: string
  requested: number
  state: ReproductionState
  /** `sampling` or `classifying` while running. */
  phase?: string
  samples: Reply[]
  original?: Reply
  fidelity?: Fidelity
  cost_usd?: number
  judge_input_tokens: number
  judge_output_tokens: number
  by: string
  started_at: number
  updated_at: number
  finished_at?: number
  error?: string
}

export type ReproduceChange = { kind: 'none' } | { kind: 'proposed' } | { kind: 'custom'; edits: ChangeEdit[] }

export interface ReproducePreview {
  fidelity: Fidelity
  original: Reply
  model: string
  provider?: string
  step_cost_usd?: number
  samples: number
}

export interface ReproduceResponse {
  preview?: ReproducePreview
  reproduction_id?: string
  review?: SuggestionReview
}
