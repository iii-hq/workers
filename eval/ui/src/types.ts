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
  revision: string
  updated_at: number
  enabled_since?: number
}

export interface CapacityRejection {
  session_id: string
  turn_id: string
  at: number
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
  /** Triage confidence under which a session is investigated anyway. */
  low_confidence: number
  /** Percent of sessions with no signal that are investigated anyway. */
  audit_sample_percent: number
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
}

export interface Failure {
  stage: AnalysisStatus
  code: string
  message: string
}

export type CoverageLevel = 'complete' | 'partial' | 'insufficient'

export type RoutingReason =
  | 'diagnostics'
  | 'needs_investigation'
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
}

export interface E2eExecution {
  execution_id: string
  label?: string
  status?: string
  conclusion?: string
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

/** `eval::propose-validation`: Jev's pick among the comparable E2E pairs. */
export type ProposalOutcome = 'proposed' | 'none_fits' | 'no_comparable_pair'

export interface ValidationProposal {
  baseline_execution_id: string
  candidate_execution_id: string
  /** Jev's distribution over the offered pairs, not whether the pair is right. */
  confidence: number
  low_confidence: boolean
  /** Computed in code: "recorded stacks identical", "recorded stack differs: …" or "… unknown". */
  stack_note: string
}

export interface ProposalAlternative {
  baseline_execution_id: string
  candidate_execution_id: string
  probability: number
  stack_note: string
}

export interface ProposeValidationResponse {
  outcome: ProposalOutcome
  proposal?: ValidationProposal
  runs_listed: number
  runs_considered: number
  pairs_considered: number
  pairs_dropped: number
  /** Runs left out, by E2E status, `other_scenario` or `no_id`. */
  excluded: Record<string, number>
  alternatives: ProposalAlternative[]
  jev?: { model: string; request_id: string; stats: Stats }
}

export interface AnalysisAssets {
  evaluation_id: string
  snapshot?: Snapshot
  triage?: Triage
  triage_failure?: TriageFailure
  investigation?: Investigation
  validations: ValidationLink[]
}

export interface AnalysisResult {
  record: AnalysisRecord
  assets: AnalysisAssets
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
}
