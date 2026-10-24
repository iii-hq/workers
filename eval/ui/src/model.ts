// Pure presentation of a cycle record. Every value comes from the record;
// anything the backend did not record is reported as missing, never guessed.
import { formatDuration } from '@iii-dev/console-ui/format'
import type {
  ActorStatus,
  Criterion,
  CriterionMetric,
  CycleRecord,
  CycleStage,
  CycleStatus,
  DecisionOutcome,
  Evidence,
  EvidenceSide,
  FindingClass,
  LoopConfig,
  OutcomeCode,
  PromotionJournal,
  PromotionPhase,
  SentinelResult,
  SentinelTriage,
  ValidationOutcome,
} from './types'

export type Tone = 'neutral' | 'accent' | 'ok' | 'warn' | 'alert'

/** Not terminal: the cycle can still move, and be cancelled. */
export const isActive = (status: CycleStatus) => status === 'queued' || status === 'running' || status === 'waiting'

export function cycleInput(record: CycleRecord): { kind: 'objective' | 'execution'; value: string } {
  return 'objective' in record.input
    ? { kind: 'objective', value: record.input.objective }
    : { kind: 'execution', value: record.input.execution_id }
}

export function cycleTitle(record: CycleRecord): string {
  const input = cycleInput(record)
  return input.kind === 'objective' ? input.value : `Execution ${input.value}`
}

const STATUS: Record<CycleStatus, { label: string; tone: Tone }> = {
  queued: { label: 'Queued', tone: 'neutral' },
  running: { label: 'Running', tone: 'accent' },
  waiting: { label: 'Waiting', tone: 'accent' },
  completed: { label: 'Completed', tone: 'neutral' },
  failed: { label: 'Failed', tone: 'alert' },
  cancelled: { label: 'Cancelled', tone: 'neutral' },
}

export const statusView = (status: CycleStatus) => STATUS[status]

const STAGE: Record<CycleStage, string> = {
  coverage: 'Coverage',
  baseline: 'Baseline',
  awaiting_execution: 'Awaiting execution',
  analysis: 'Analysis',
  triage: 'Triage',
  proposal: 'Proposal',
  candidate: 'Candidate',
  validation: 'Validation',
  promotion: 'Promotion',
  done: 'Done',
}

export const stageLabel = (stage: CycleStage) => STAGE[stage]

/**
 * The stages the cycle actually went through, from its history, oldest
 * first: the path is not fixed (an execution may go analysis, triage, then
 * coverage; a validated candidate returns to analysis).
 */
export function visitedStages(record: CycleRecord): CycleStage[] {
  const stages: CycleStage[] = []
  for (const event of record.history) if (stages.at(-1) !== event.stage) stages.push(event.stage)
  if (stages.at(-1) !== record.stage) stages.push(record.stage)
  return stages
}

const DECISION: Record<DecisionOutcome, { label: string; tone: Tone }> = {
  // Approval of the candidate only: not a merge, publication or activation.
  accepted: { label: 'Accepted', tone: 'ok' },
  rejected: { label: 'Rejected', tone: 'alert' },
  inconclusive: { label: 'Inconclusive', tone: 'warn' },
}

const OUTCOME: Record<OutcomeCode, { label: string; tone: Tone }> = {
  no_coverage: { label: 'No coverage', tone: 'warn' },
  no_action: { label: 'No action', tone: 'neutral' },
  budget: { label: 'Budget reached', tone: 'warn' },
  inconclusive: { label: 'Inconclusive', tone: 'warn' },
  decided: { label: 'Decided', tone: 'neutral' },
  promoted: { label: 'Promoted', tone: 'ok' },
  rolled_back: { label: 'Rolled back', tone: 'warn' },
}

export type Progress = 'not_requested' | 'requested' | 'confirmed' | 'unknown'

export interface PromotionSummary {
  merge: Progress
  release: Progress
  activation: Progress
  /** `confirmed` means the prior executable is proved running again. */
  rollback: Progress
  stopped?: Extract<PromotionPhase, { phase: 'stopped' }>
}

const has = (journal: PromotionJournal | undefined, phase: PromotionPhase['phase']) =>
  journal?.phases.some((entry) => entry.phase === phase) ?? false

/**
 * Requested and confirmed apart, per effect, from the journal only. A step
 * requested and then stopped as unavailable or inconclusive is `unknown`:
 * the effect may or may not have happened.
 */
export function promotionSummary(journal: PromotionJournal | undefined): PromotionSummary {
  const stopped = [...(journal?.phases ?? [])]
    .reverse()
    .find((entry): entry is Extract<PromotionPhase, { phase: 'stopped' }> => entry.phase === 'stopped')
  const uncertain = stopped !== undefined && stopped.kind !== 'rejected'
  const progress = (requested: boolean, confirmed: boolean): Progress =>
    confirmed ? 'confirmed' : requested ? (uncertain ? 'unknown' : 'requested') : 'not_requested'
  return {
    merge: progress(has(journal, 'merge_requested'), has(journal, 'merged')),
    release: progress(
      has(journal, 'rc_planned') ||
        has(journal, 'rc_confirmed') ||
        has(journal, 'finalize_planned') ||
        has(journal, 'finalize_confirmed'),
      has(journal, 'stable_published'),
    ),
    activation: progress(has(journal, 'activation_requested'), has(journal, 'activation_confirmed')),
    rollback: progress(has(journal, 'rollback_requested'), has(journal, 'rollback_confirmed')),
    stopped,
  }
}

/**
 * The cycle's headline outcome. A completed status alone is never a
 * decision, an accepted decision is never "Promoted", and the backend's
 * `promoted`/`rolled_back` claims are shown only with the journal's
 * confirmation of activation/rollback (otherwise unconfirmed).
 */
export function outcomeView(record: CycleRecord): { label: string; tone: Tone; reason: string } | null {
  const outcome = record.outcome
  if (outcome?.code === 'promoted' || outcome?.code === 'rolled_back') {
    const summary = promotionSummary(record.promotion)
    if (outcome.code === 'rolled_back') {
      if (summary.rollback === 'confirmed') return { ...OUTCOME.rolled_back, reason: outcome.detail }
      return {
        label: 'Rollback unconfirmed',
        tone: 'warn',
        reason: `${outcome.detail} (the promotion journal holds no rollback confirmation)`,
      }
    }
    // Promoted needs both facts: the executable proved running and the canary held.
    const missing = [
      summary.activation === 'confirmed' ? null : 'no activation confirmation in the journal',
      record.canary?.outcome === 'held' ? null : `canary ${record.canary?.outcome ?? 'not recorded'}`,
    ].filter(Boolean)
    if (missing.length === 0) return { ...OUTCOME.promoted, reason: outcome.detail }
    return { label: 'Promotion unconfirmed', tone: 'warn', reason: `${outcome.detail} (${missing.join('; ')})` }
  }
  if (outcome && outcome.code !== 'decided') return { ...OUTCOME[outcome.code], reason: outcome.detail }
  if (record.decision) return { ...DECISION[record.decision.outcome], reason: record.decision.reason }
  if (outcome) return { ...OUTCOME.decided, reason: outcome.detail }
  if (record.status === 'failed') {
    return { label: 'Failed', tone: 'alert', reason: record.failure ?? 'No failure reason recorded.' }
  }
  if (record.status === 'completed') {
    return { label: 'No decision', tone: 'neutral', reason: 'The cycle completed without recording an outcome.' }
  }
  return null
}

export const decisionView = (outcome: DecisionOutcome) => DECISION[outcome]

/** An actor's status line: the state and its reason or code. */
export function actorStatusLine(status: ActorStatus): string {
  switch (status.state) {
    case 'completed':
      return 'completed'
    case 'failed':
      return `failed (${status.code}): ${status.reason}`
    default:
      return `${status.state.replace('_', ' ')}: ${status.reason}`
  }
}

/** What the cycle is doing now, or what it waits for, from the record itself. */
export function nextStage(record: CycleRecord): string {
  const stage = stageLabel(record.stage)
  switch (record.status) {
    case 'queued':
      return `Queued at ${stage}.`
    case 'running':
      return `${stage}: ${record.history.at(-1)?.detail ?? 'running'}`
    case 'waiting':
      return record.next ? `${stage}: ${record.next.detail}` : `${stage}: waiting on an external event.`
    case 'cancelled':
      return `Cancelled at ${stage}; nothing further runs.`
    case 'failed':
      return `Failed at ${stage}; nothing further runs.`
    case 'completed':
      return 'Completed; nothing further runs.'
  }
}

export type GapKind = 'no_coverage' | 'unknown' | 'inconclusive'
export interface Gap {
  kind: GapKind
  subject: string
  reason: string
}

function sideGaps(subject: string, label: string, side: EvidenceSide): Gap[] {
  if (side.runs.length === 0) {
    return [
      { kind: 'no_coverage', subject, reason: `${label} execution ${side.execution_id} holds no run of the scenario.` },
    ]
  }
  if (side.n === 0) {
    const broken = side.runs.filter((run) => run.technical?.toLowerCase() !== 'valid').length
    return [
      {
        kind: 'no_coverage',
        subject,
        reason: `${label} has ${side.runs.length} run(s), none measurable${broken ? ` (${broken} without a valid technical outcome)` : ''}.`,
      },
    ]
  }
  if (side.mean === undefined)
    return [{ kind: 'unknown', subject, reason: `${label} reported no value of the metric.` }]
  return []
}

function evidenceGaps(subject: string, evidence: Evidence): Gap[] {
  const found = [
    ...sideGaps(subject, 'Baseline', evidence.baseline),
    ...sideGaps(subject, 'Candidate', evidence.candidate),
  ]
  if (evidence.computed_outcome === 'inconclusive')
    found.push({ kind: 'inconclusive', subject, reason: evidence.reason })
  return found
}

/** Every no_coverage, unknown and inconclusive reason the record carries. */
export function gaps(record: CycleRecord): Gap[] {
  const found: Gap[] = []
  const coverage = record.coverage
  if (coverage && !coverage.covered) {
    for (const reason of coverage.gaps) found.push({ kind: 'no_coverage', subject: `Need · ${coverage.need}`, reason })
    if (coverage.gaps.length === 0) {
      found.push({ kind: 'no_coverage', subject: `Need · ${coverage.need}`, reason: 'No E2E test covers it.' })
    }
  }
  for (const analysis of record.analyses ?? []) {
    const subject = `${analysis.role} · ${analysis.execution_id}`
    for (const reason of analysis.unknowns) found.push({ kind: 'unknown', subject, reason })
    if (analysis.sentinel?.error) {
      found.push({ kind: 'unknown', subject: `${subject} · Sentinel`, reason: analysis.sentinel.error })
    }
    if (!analysis.child_transcripts_complete) {
      found.push({ kind: 'unknown', subject, reason: 'Not every child transcript was kept.' })
    }
    for (const finding of analysis.findings) {
      if (finding.class === 'insufficient_evidence') {
        found.push({ kind: 'inconclusive', subject: `${subject} · ${finding.signal}`, reason: finding.observation })
      }
    }
  }
  if (record.baseline && !record.baseline.usable) {
    found.push({
      kind: 'inconclusive',
      subject: `Baseline · ${record.baseline.execution_id}`,
      reason: record.baseline.reason,
    })
  }
  const decision = record.decision
  if (decision) {
    for (const check of decision.comparability.checks) {
      if (check.baseline === undefined || check.candidate === undefined) {
        const missing =
          check.baseline === undefined
            ? check.candidate === undefined
              ? 'either side'
              : 'the baseline'
            : 'the candidate'
        found.push({
          kind: 'unknown',
          subject: `Comparability · ${check.field}`,
          reason: `Not reported by ${missing}.`,
        })
      }
    }
    found.push(...evidenceGaps('Primary criterion', decision.primary))
    for (const [index, evidence] of decision.preserved.entries()) {
      found.push(...evidenceGaps(`Preserved #${index + 1}`, evidence))
    }
  }
  if (record.artifact_decision) {
    found.push(...evidenceGaps('Artifact primary', record.artifact_decision.primary))
    for (const [index, evidence] of record.artifact_decision.preserved.entries()) {
      found.push(...evidenceGaps(`Artifact preserved #${index + 1}`, evidence))
    }
  }
  const triage = record.triage
  if (triage?.outcome === 'unknown')
    found.push({ kind: 'unknown', subject: `Triage · ${triage.source}`, reason: triage.reason })
  const proposal = record.proposal?.proposal
  if (proposal?.status.state === 'failed') {
    found.push({ kind: 'inconclusive', subject: 'Proposal', reason: actorStatusLine(proposal.status) })
  }
  const candidate = record.candidate?.candidate
  if (candidate?.status.state === 'failed') {
    found.push({ kind: 'inconclusive', subject: 'Candidate', reason: actorStatusLine(candidate.status) })
  }
  for (const [subject, actor] of [
    ['Proposal cost', proposal],
    ['Candidate cost', candidate],
  ] as const) {
    if (actor && actor.status.state !== 'not_started' && actor.cost.cost_usd === null) {
      found.push({ kind: 'unknown', subject, reason: 'harness::metrics reported no cost.' })
    }
  }
  const stopped = promotionSummary(record.promotion).stopped
  // A rejected stop is a concrete result, shown with the promotion; only an
  // unavailable or inconclusive stop leaves an effect unknown.
  if (stopped && stopped.kind !== 'rejected') {
    found.push({
      kind: stopped.kind === 'inconclusive' ? 'inconclusive' : 'unknown',
      subject: `Promotion · ${stopped.step} (${stopped.kind})`,
      reason: stopped.reason,
    })
  }
  if (record.canary?.outcome === 'unknown') {
    found.push({ kind: 'unknown', subject: `Canary · ${record.canary.execution_id}`, reason: record.canary.reason })
  }
  const outcome = record.outcome
  if (outcome && (outcome.code === 'inconclusive' || outcome.code === 'budget')) {
    found.push({ kind: 'inconclusive', subject: `Outcome · ${outcome.code}`, reason: outcome.detail })
  } else if (outcome?.code === 'no_coverage' && !found.some((gap) => gap.kind === 'no_coverage')) {
    found.push({ kind: 'no_coverage', subject: 'Outcome', reason: outcome.detail })
  }
  if (decision?.outcome === 'inconclusive' && !found.some((gap) => gap.kind === 'inconclusive')) {
    found.push({ kind: 'inconclusive', subject: 'Decision', reason: decision.reason })
  }
  return found
}

const FINDING: Record<FindingClass, { label: string; tone: Tone }> = {
  defect: { label: 'Defect', tone: 'alert' },
  improvement_opportunity: { label: 'Improvement', tone: 'accent' },
  evaluation_gap: { label: 'Evaluation gap', tone: 'warn' },
  infrastructure_or_evaluator_failure: { label: 'Infra/evaluator', tone: 'warn' },
  expected_behavior: { label: 'Expected', tone: 'neutral' },
  insufficient_evidence: { label: 'Insufficient evidence', tone: 'warn' },
}

export const findingView = (findingClass: FindingClass) => FINDING[findingClass]

const METRIC: Record<CriterionMetric, string> = {
  signal_per_run: 'signal per run',
  pass_rate: 'pass rate',
  criterion_score: 'criterion score',
  cost_usd: 'cost (USD)',
  duration: 'wall time',
  tokens: 'tokens',
  function_calls: 'function calls',
}

const trim = (value: number) => (Number.isInteger(value) ? String(value) : value.toFixed(2))

/** A mean or delta in the metric's own unit: a fraction for rates, ms for wall time. */
export function formatMetric(metric: CriterionMetric, value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value)) return '—'
  switch (metric) {
    case 'pass_rate':
    case 'criterion_score':
      return `${(value * 100).toFixed(1)}%`
    case 'cost_usd':
      return `$${value.toFixed(4)}`
    case 'duration':
      return formatDuration(value)
    default:
      return trim(value)
  }
}

/** `min_effect` in its own unit: pp for rates, seconds for wall time. */
export function formatEffect(metric: CriterionMetric, value: number): string {
  switch (metric) {
    case 'pass_rate':
    case 'criterion_score':
      return `${trim(value)} pp`
    case 'duration':
      return `${trim(value)} s`
    case 'cost_usd':
      return `$${trim(value)}`
    default:
      return trim(value)
  }
}

export function criterionLine(criterion: Criterion): string {
  const target = criterion.pattern ?? criterion.criterion_id
  const metric = target ? `${METRIC[criterion.metric]} of ${target}` : METRIC[criterion.metric]
  return `${metric} must ${criterion.direction} by at least ${formatEffect(criterion.metric, criterion.min_effect)}, ≥ ${criterion.min_runs} runs each side, on ${criterion.scenario_id}`
}

/** Candidate minus baseline; null when either mean is missing. */
export function delta(evidence: Evidence): { absolute: number; relative: number | null } | null {
  const { baseline, candidate } = evidence
  if (baseline.mean === undefined || candidate.mean === undefined) return null
  const absolute = candidate.mean - baseline.mean
  return { absolute, relative: baseline.mean === 0 ? null : absolute / Math.abs(baseline.mean) }
}

export function deltaLine(metric: CriterionMetric, evidence: Evidence): string {
  const change = delta(evidence)
  if (!change) return 'unknown'
  const sign = change.absolute > 0 ? '+' : change.absolute < 0 ? '−' : ''
  const magnitude =
    metric === 'pass_rate' || metric === 'criterion_score'
      ? `${(Math.abs(change.absolute) * 100).toFixed(1)} pp`
      : formatMetric(metric, Math.abs(change.absolute))
  const relative = change.relative === null ? '' : ` (${sign}${Math.abs(change.relative * 100).toFixed(1)}%)`
  return `${sign}${magnitude}${relative}`
}

const VALIDATION: Record<ValidationOutcome, { label: string; tone: Tone }> = {
  validated_improvement: { label: 'Improved', tone: 'ok' },
  no_improvement: { label: 'No improvement', tone: 'neutral' },
  regression: { label: 'Regression', tone: 'alert' },
  inconclusive: { label: 'Inconclusive', tone: 'warn' },
}

export const validationView = (outcome: ValidationOutcome) => VALIDATION[outcome]

const text = (value: unknown) => (typeof value === 'string' && value ? value : undefined)
const count = (value: unknown) => (typeof value === 'number' && Number.isFinite(value) ? value : undefined)

function sentinelTriage(value: unknown): SentinelTriage | undefined {
  if (!value || typeof value !== 'object') return undefined
  const triage = value as Record<string, unknown>
  const kind = text(triage.kind)
  if (!kind) return undefined
  const usage = triage.usage && typeof triage.usage === 'object' ? (triage.usage as Record<string, unknown>) : undefined
  return {
    kind,
    confidence: count(triage.confidence),
    source: text(triage.source),
    model: text(triage.model),
    question: text(triage.question),
    criterion: text(triage.criterion),
    at_ms: count(triage.at_ms),
    usage: usage ? { input_tokens: count(usage.input_tokens), output_tokens: count(usage.output_tokens) } : undefined,
  }
}

/**
 * One of Sentinel's `results[]`, which the eval contract passes through as
 * JSON: only the fields Sentinel actually returned, `null` when the entry is
 * not a result at all.
 */
export function sentinelResult(value: unknown): SentinelResult | null {
  if (!value || typeof value !== 'object') return null
  const result = value as Record<string, unknown>
  const dedupeKey = text(result.dedupe_key)
  const classification = text(result.classification)
  const outcome = text(result.outcome)
  if (!dedupeKey || !classification || !outcome) return null
  return {
    dedupe_key: dedupeKey,
    classification,
    outcome,
    group_id: text(result.group_id),
    occurrence_id: text(result.occurrence_id),
    group_status: text(result.group_status),
    triage: sentinelTriage(result.triage),
  }
}

/** The same rules as `LoopConfigV1::validate`, so a save is refused before the call. */
export function validateConfig(config: LoopConfig): string | null {
  const positive = (value: number) => Number.isFinite(value) && value > 0
  if (!config.subject.provider.trim() || !config.subject.model.trim()) {
    return 'subject.provider and subject.model must name a model'
  }
  if (config.where === 'harness' && config.stack) return 'where harness runs on its own stack: omit stack'
  if (config.where !== 'harness' && !config.stack) return 'where docker or github needs a stack {name, yaml}'
  if (config.stack && (!config.stack.name.trim() || !config.stack.yaml.trim())) {
    return 'stack.name and stack.yaml must not be empty'
  }
  const whole = (value: number, min: number) => Number.isInteger(value) && value >= min
  if (!whole(config.runs_per_side, 1) || !whole(config.max_scenarios, 1)) {
    return 'runs_per_side and max_scenarios must be whole numbers of at least 1'
  }
  if (!whole(config.technical_retries, 0) || config.technical_retries > 255) {
    return 'technical_retries must be a whole number from 0 to 255'
  }
  if (!positive(config.estimated_usd_per_run)) return 'estimated_usd_per_run must be a positive number of US dollars'
  if (!positive(config.min_effect.tokens) || !positive(config.min_effect.pass_rate_pp)) {
    return 'min_effect.tokens and min_effect.pass_rate_pp must be positive'
  }
  const budget = config.budget
  if (
    !positive(budget.max_spend_usd) ||
    !whole(budget.max_duration_ms, 1) ||
    !whole(budget.max_calls, 1) ||
    !whole(budget.max_candidates, 1)
  ) {
    return 'every budget limit must be positive'
  }
  if (config.controls?.some((control) => !control.trim())) return 'controls must name scenarios'
  const judge = config.judge
  if (judge && (!judge.provider.trim() || !Number.isFinite(judge.estimated_usd) || judge.estimated_usd < 0)) {
    return 'judge.provider must be set and judge.estimated_usd be 0 or more'
  }
  const candidate = config.candidate
  if (candidate) {
    // eval/src/candidate.rs `validate_config`.
    if (!/^[0-9a-f]{40}$/.test(candidate.base_sha)) return 'candidate.base_sha must be a full lowercase 40-hex commit'
    if (!candidate.repository.startsWith('/') || !candidate.worktrees_dir.startsWith('/')) {
      return 'candidate.repository and candidate.worktrees_dir must be absolute paths'
    }
    if (!candidate.model.trim() || !candidate.provider.trim())
      return 'candidate.model and candidate.provider must be set'
    if (
      !whole(candidate.max_turns, 1) ||
      !whole(candidate.max_output_tokens, 1) ||
      !whole(candidate.max_total_tokens, 1) ||
      !positive(candidate.max_cost_usd)
    ) {
      return 'candidate max_turns, max_output_tokens, max_total_tokens and max_cost_usd must be positive'
    }
  }
  const validation = config.validation
  if (validation) {
    if (!candidate) return 'validation needs candidate: only a candidate is validated'
    if (validation.where === 'harness') return 'validation pins the Harness: where must be docker or github'
    if (
      !validation.source_stack_template.includes('{commit}') ||
      !validation.artifact_stack_template.includes('{version}') ||
      !validation.stack_name.trim()
    ) {
      return 'validation needs a stack_name and templates with {commit} and {version}'
    }
  }
  const promotion = config.promotion
  if (promotion) {
    if (!validation) return 'promotion needs validation: an unvalidated artifact is never released'
    if (!positive(config.release_build_estimate_usd ?? Number.NaN)) {
      return 'promotion needs a positive release_build_estimate_usd'
    }
    const target = [
      promotion.artifact_target,
      promotion.target.engine_url,
      promotion.target.namespace,
      promotion.target.compose_file,
    ]
    if (target.some((value) => !value.trim())) return 'promotion needs the artifact target and the activation target'
  }
  return null
}

/** Parses a page context from `host.panels.open`: `{ type: 'cycle', cycleId }`, `{ type: 'start' }` or `{ type: 'config' }`. */
export function parseOpenContext(context: unknown): { cycleId?: string; start?: boolean; config?: boolean } | null {
  if (!context || typeof context !== 'object') return null
  const value = context as { type?: unknown; cycleId?: unknown }
  if (value.type === 'cycle' && typeof value.cycleId === 'string' && value.cycleId) return { cycleId: value.cycleId }
  if (value.type === 'start') return { start: true }
  if (value.type === 'config') return { config: true }
  return null
}
