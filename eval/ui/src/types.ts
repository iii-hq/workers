// The loop contract of `eval::configure|config|start|list|status|result|cancel|executions`,
// mirrored from eval/src/contract.rs. Optional fields are absent when the
// backend did not record them: never zero, never guessed.

// ── configuration ──────────────────────────────────────────────────────────

export type Where = 'harness' | 'docker' | 'github'

export interface Subject {
  provider: string
  model: string
  agent?: string
}

export interface Stack {
  name: string
  yaml: string
}

export interface CycleBudget {
  max_spend_usd: number
  max_duration_ms: number
  max_calls: number
  max_candidates: number
}

export interface LoopConfig {
  subject: Subject
  where: Where
  /** Required for docker and github; refused for harness. */
  stack?: Stack
  runs_per_side: number
  technical_retries: number
  max_scenarios: number
  estimated_usd_per_run: number
  min_effect: { tokens: number; pass_rate_pp: number }
  budget: CycleBudget
  /** Control scenarios every plan also runs and preserves. */
  controls?: string[]
  /** Semantic triage of ambiguous findings; absent: they stay inconclusive. */
  judge?: JudgeConfig
  /** The read-only proposal and the isolated candidate actor; absent: no proposal. */
  candidate?: CandidateConfig
  /** Stacks pinning the Harness under test; needs `candidate`. */
  validation?: ValidationConfig
}

export interface JudgeConfig {
  provider: string
  estimated_usd: number
}

export type ThinkingLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh'

/** `CandidateConfigV1` (eval/src/candidate.rs). */
export interface CandidateConfig {
  repository: string
  base_sha: string
  worktrees_dir: string
  model: string
  provider: string
  thinking_level?: ThinkingLevel
  max_turns: number
  max_output_tokens: number
  max_total_tokens: number
  max_cost_usd: number
  publish_remote?: string
}

export interface ValidationConfig {
  where: Where
  stack_name: string
  /** `{commit}` is replaced by the 40-hex sha. */
  source_stack_template: string
}

export interface StoredConfig {
  config: LoopConfig
  /** The `policy_revision` every plan freezes. */
  revision: string
  updated_at: number
}

export interface ConfigResponse {
  config: StoredConfig | null
  rules_version: string
}

// ── the cycle ───────────────────────────────────────────────────────────────

export type CycleStatus = 'queued' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled'
export type CycleStage =
  | 'coverage'
  | 'baseline'
  | 'awaiting_execution'
  | 'analysis'
  | 'triage'
  | 'insight'
  | 'proposal'
  | 'candidate'
  | 'validation'
  | 'pull_request'
  | 'done'

/** Exactly one of the two entries; serde's externally tagged `CycleInputV1`. */
export type CycleInput = { objective: string } | { execution_id: string }

export interface CycleEvent {
  at: number
  status: CycleStatus
  stage: CycleStage
  detail: string
}

/** `control`: a round of the base run beside the candidate's, what the candidate is compared with. */
export type ExecutionRole = 'source' | 'baseline' | 'candidate' | 'control'
export type ExecutionState = 'dispatching' | 'running' | 'imported' | 'lost'

export interface CycleExecution {
  role: ExecutionRole
  state: ExecutionState
  execution_id?: string
  label?: string
  work_id?: string
  dispatched_at?: number
  admission_key?: string
  detail?: string
}

export interface TestCriterion {
  id: string
  description: string
  gate: boolean
}

export interface SelectedTest {
  scenario_id: string
  case_id: string
  seed: number
  behavior_sha256: string
  contract_sha256: string
  inputs_sha256: string
  criteria: TestCriterion[]
  invariants: string[]
  matched_terms: string[]
  rationale: string
  history_note?: string
}

export interface Coverage {
  need: string
  covered: boolean
  selected: SelectedTest[]
  gaps: string[]
  tests_total: number
  tests_revision: string
  catalog_sha256: string
  selected_at: number
}

export type FindingClass =
  | 'defect'
  | 'improvement_opportunity'
  | 'evaluation_gap'
  | 'infrastructure_or_evaluator_failure'
  | 'expected_behavior'
  | 'insufficient_evidence'

export interface Finding {
  fingerprint: string
  class: FindingClass
  signal: string
  observation: string
}

export interface ScenarioIdentity {
  scenario_id: string
  case_id?: string
  behavior_sha256?: string
  inputs_sha256?: string
}

/** `null` is unknown: absent, or reports that disagree. */
export interface EvidenceIdentity {
  scenarios?: ScenarioIdentity[]
  contract_digest: string | null
  artifacts_digest: string | null
  stack_digest: string | null
  model: string | null
  provider: string | null
  harness_version: string | null
  workers_revision: string | null
  engine_version: string | null
  e2e_revision: string | null
  /** SHA-256 of the running Harness executable, as the E2E runner captured it. */
  harness_binary_sha256?: string | null
  /** Digest of the native subject configuration (model, provider, agent). */
  subject_sha256?: string | null
  /** Digest of every stack component but the Harness. */
  fixed_stack_digest?: string | null
}

export interface CycleAnalysis {
  execution_id: string
  admission_key: string
  role: ExecutionRole
  identity: EvidenceIdentity
  findings: Finding[]
  unknowns: string[]
  child_transcripts_complete: boolean
  /** One entry per case the execution ran, with its run count. */
  scenarios?: { scenario_id: string; runs: number }[]
  /** What `sentinel::reports::ingest` answered; absent when the reports recorded no failure. */
  sentinel?: SentinelForward
  analyzed_at: number
}

/** `SentinelForwardV1`: the runs' recorded failures forwarded to Sentinel, kept as it answered. */
export interface SentinelForward {
  observations: number
  recorded?: number
  deduped?: number
  skipped?: number
  /** Sentinel's `ReportObservationResultV1`, untyped on the eval side: read with `sentinelResult`. */
  results?: unknown[]
  error?: string
}

/** Sentinel's `GroupTriageV1`: its own classification, never the loop's decision. */
export interface SentinelTriage {
  kind: string
  confidence?: number
  source?: string
  model?: string
  question?: string
  criterion?: string
  at_ms?: number
  usage?: { input_tokens?: number; output_tokens?: number }
}

/** Sentinel's `ReportObservationResultV1`. */
export interface SentinelResult {
  dedupe_key: string
  classification: string
  outcome: string
  group_id?: string
  occurrence_id?: string
  group_status?: string
  /** Absent until Sentinel's triage sweep reaches the group. */
  triage?: SentinelTriage
}

export type CriterionMetric =
  | 'signal_per_run'
  | 'pass_rate'
  | 'criterion_score'
  | 'cost_usd'
  | 'duration'
  | 'tokens'
  | 'function_calls'
  | 'function_call_errors'
export type Direction = 'decrease' | 'increase'

export interface Criterion {
  metric: CriterionMetric
  pattern?: string
  criterion_id?: string
  direction: Direction
  /** In the metric's effect unit: pp for pass rate and criterion score, s for duration. */
  min_effect: number
  min_runs: number
  scenario_id: string
}

export interface EvidenceRun {
  run_id: string
  case_id?: string
  completed: boolean
  completion?: string
  technical?: string
  status?: string
  cost_usd?: number
  wall_time_ms?: number
  total_tokens?: number
  function_calls?: number
  function_call_errors?: number
  criteria?: Record<string, number>
  signals?: Record<string, number>
}

export interface EvidenceSide {
  execution_id: string
  runs: EvidenceRun[]
  n: number
  mean?: number
}

export interface Baseline {
  execution_id: string
  primary: EvidenceSide
  preserved: EvidenceSide[]
  usable: boolean
  reason: string
}

export interface CycleNext {
  action: 'await_execution' | 'candidate_work'
  detail: string
  findings?: string[]
}

export type OutcomeCode = 'no_coverage' | 'no_action' | 'budget' | 'inconclusive' | 'decided' | 'proposed'

export interface CycleOutcome {
  code: OutcomeCode
  detail: string
}

export interface CyclePlan {
  objective: string
  /** The selected tests and controls, each with the identity of its current definition. */
  tests: SelectedTest[]
  criterion: Criterion
  preserved?: Criterion[]
  controls?: string[]
  sampling: { runs_per_side: number; seeds?: string[] }
  parameters: {
    provider: string
    model: string
    runs: number
    scenarios: string[]
    technical_retries: number
    where: Where
    stack?: Stack
    agent?: string
  }
  /** What the baseline is reserved at. */
  estimated_usd: number
  policy_revision: string
  frozen_at: number
}

export type ValidationOutcome = 'validated_improvement' | 'no_improvement' | 'regression' | 'inconclusive'

export interface Evidence {
  scenario_id: string
  computed_at: number
  baseline: EvidenceSide
  candidate: EvidenceSide
  computed_outcome: ValidationOutcome
  reason: string
}

export interface ComparabilityCheck {
  field: string
  baseline?: string
  candidate?: string
  matches: boolean
}

export type DecisionOutcome = 'accepted' | 'rejected' | 'inconclusive'

export interface CycleDecision {
  outcome: DecisionOutcome
  reason: string
  policy_revision: string
  baseline_execution_id: string
  candidate_execution_id: string
  comparability: { comparable: boolean; checks: ComparabilityCheck[] }
  primary: Evidence
  preserved: Evidence[]
  decided_at: number
}

export interface CycleRecord {
  schema_version: number
  cycle_id: string
  input: CycleInput
  status: CycleStatus
  stage: CycleStage
  step: number
  created_at: number
  updated_at: number
  completed_at?: number
  history: CycleEvent[]
  coverage?: Coverage
  plan?: CyclePlan
  executions?: CycleExecution[]
  analyses?: CycleAnalysis[]
  baseline?: Baseline
  next?: CycleNext
  decision?: CycleDecision
  outcome?: CycleOutcome
  failure?: string
  /** The configuration frozen at creation; `null`/absent: created unconfigured. */
  config?: StoredConfig
  triage?: Triage
  proposal?: ProposalState
  candidate?: CandidateState
  /** The accepted candidate's pull request, once opened or found. */
  pull_request?: PullRequest
  /** The ended cycle this one measures again, from its finding. */
  retry_of?: string
  /** What the measured runs' transcripts show, read before the proposal. */
  insight?: Insight
}

/** `SiteClassV1`: whether a Harness change, instructing the model included, could prevent a failed call. */
export type SiteClass = 'preventable' | 'one_off' | 'task' | 'unclear'

/** `ErrorSiteV1`: one failed function call of a measured run. */
export interface ErrorSite {
  site_id: string
  execution_id: string
  scenario_id: string
  run_id: string
  entry_id: string
  function_id: string
  arguments: string
  error: string
  before: string
  after: string
  step?: string
  class?: SiteClass
  probability?: number
}

/** `ImprovementPointV1`: a Harness change worth trying, with the sites that show it. */
export interface ImprovementPoint {
  function_id: string
  summary: string
  sites: string[]
  source: 'jev' | 'llm'
}

/** `InsightV1`. */
export interface Insight {
  sites: ErrorSite[]
  points?: ImprovementPoint[]
  source?: 'jev' | 'llm'
  reason: string
  model?: string
}

/** `PullRequestV1` (eval/src/pull_request.rs). `created`: opened by this cycle rather than found. */
export interface PullRequest {
  number: number
  url: string
  head_ref: string
  head_sha: string
  created: boolean
}

// ── triage, proposal, candidate ──

export type TriageSource = 'objective' | 'deterministic' | 'sentinel' | 'jev'
export type TriageOutcome = 'proceed' | 'no_action' | 'unknown'

/** Why the cycle proceeds past its findings, or stops. `question`, `answer` and `usage` are the judge's JSON. */
export interface Triage {
  source: TriageSource
  outcome: TriageOutcome
  findings: string[]
  reason: string
  request_id?: string
  question?: unknown
  evidence_digest?: string
  model?: string
  answer?: unknown
  usage?: unknown
  /** Sentinel triage of the same occurrences, as Sentinel answered. */
  reused?: unknown[]
}

/** `CandidateStatusV1`, tagged by `state`. */
export type ActorStatus =
  | { state: 'not_started'; reason: string }
  | { state: 'pending'; reason: string }
  | { state: 'completed' }
  | { state: 'failed'; code: string; reason: string }

/** Measured by `harness::metrics`; `null` is unknown, never zero. */
export interface ActorCost {
  cost_usd: number | null
  input_tokens: number | null
  output_tokens: number | null
}

export interface ProposalIdentity {
  proposal_id: string
  cycle_id: string
  base_sha: string
  input_digest: string
  worktree: string
  root: string
  session_id: string
  idempotency_key: string
}

export interface ProposalCitation {
  path: string
  line_start: number
  line_end: number
}

export interface Proposal {
  identity: ProposalIdentity
  input: {
    case_ids: string[]
    objective: string
    occurrence: string
    evidence: { source: string; excerpt: string }[]
    criteria: string[]
  }
  model: string
  provider: string
  status: ActorStatus
  turn_id: string | null
  cost: ActorCost
  hypothesis: string | null
  change: string | null
  citations: ProposalCitation[]
}

export interface ProposalState {
  identity: ProposalIdentity
  proposal?: Proposal
}

export interface CandidateIdentity {
  candidate_id: string
  cycle_id: string
  base_sha: string
  brief_digest: string
  branch: string
  worktree: string
  root: string
  session_id: string
  idempotency_key: string
}

export interface CandidateCommit {
  base_sha: string
  candidate_sha: string
  paths: { path: string; change: string }[]
  /** What runs the change: worker names, `kits/<name>`, other top-level paths. */
  units?: string[]
  /** Other units whose build reads the changed code. */
  dependents?: string[]
  patch: string
  patch_digest: string
}

export interface Candidate {
  identity: CandidateIdentity
  status: ActorStatus
  turn_id: string | null
  cost: ActorCost
  commit: CandidateCommit | null
}

export interface CandidateState {
  identity: CandidateIdentity
  candidate?: Candidate
  candidate_tree?: string
  publish?: { remote: string; branch: string; candidate_sha: string; builder_ref: string }
}

/** What a cycle has drawn on its budget. */
export interface CycleUsage {
  budget: CycleBudget
  /** Reported costs plus the estimate of work without one. */
  spend_usd: number
  /** Work settled without a reported cost. */
  unknown_cost: number
  calls: number
  candidates: number
  elapsed_ms: number
}

export interface CycleStartResponse {
  cycle_id: string
  status: CycleStatus
}

export interface CycleResultResponse {
  cycle: CycleRecord
  /** `null` until the cycle reserves work. */
  usage: CycleUsage | null
}

export interface CycleCancelResponse {
  cancelled: boolean
  status: CycleStatus
}

export interface CycleListResponse {
  cycles: CycleRecord[]
  /** By `cycle_id`; a cycle that reserved no work is absent. */
  usage: Record<string, CycleUsage>
}

/** `ExecutionOptionV1`: a finished E2E execution a cycle can start from. */
export interface ExecutionOption {
  execution_id: string
  status: string
  started_at?: string
  /** The cycle that already imported it. */
  cycle_id?: string
  /** Empty when it is in a cycle or its reports could not be read. */
  scenarios: { scenario_id: string; mean_score?: number; lost?: string }[]
}
