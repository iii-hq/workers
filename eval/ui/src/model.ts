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
  EvidenceRun,
  EvidenceSide,
  ExecutionRole,
  FindingClass,
  LoopConfig,
  OutcomeCode,
  SentinelResult,
  SentinelTriage,
} from './types'

export type Tone = 'neutral' | 'accent' | 'ok' | 'warn' | 'alert'

/** Not terminal: the cycle can still move, and be cancelled. */
export const isActive = (status: CycleStatus) => status === 'queued' || status === 'running' || status === 'waiting'

export function cycleInput(record: CycleRecord): { kind: 'objective' | 'execution'; value: string } {
  return 'objective' in record.input
    ? { kind: 'objective', value: record.input.objective }
    : { kind: 'execution', value: record.input.execution_id }
}

/**
 * A name a person reads: the objective, or for an execution what the frozen
 * plan restores, else the scenarios it ran.
 */
export function cycleTitle(record: CycleRecord): string {
  const input = cycleInput(record)
  if (input.kind === 'objective') return input.value
  const criterion = record.plan?.criterion
  if (criterion) return `Restore ${criterionName(criterion)} on ${criterion.scenario_id}`
  const scenarios = record.analyses?.find((analysis) => analysis.role === 'source')?.identity.scenarios ?? []
  return scenarios.length && record.outcome?.code !== 'no_action'
    ? `Improve ${scenarios.map((scenario) => scenario.scenario_id).join(', ')}`
    : `Execution ${shortId(input.value)}`
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
  pull_request: 'Pull request',
  done: 'Done',
}

export const stageLabel = (stage: CycleStage) => STAGE[stage]

const DECISION: Record<DecisionOutcome, { label: string; tone: Tone }> = {
  // Approval of the candidate only: the loop opens a pull request, never merges.
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
  proposed: { label: 'Pull request opened', tone: 'ok' },
}

/**
 * The cycle's headline outcome. A completed status alone is never a
 * decision, and `proposed` is shown only with the pull request the record holds.
 */
export function outcomeView(record: CycleRecord): { label: string; tone: Tone; reason: string } | null {
  const outcome = record.outcome
  if (outcome?.code === 'proposed' && !record.pull_request) {
    return { label: 'Pull request unconfirmed', tone: 'warn', reason: `${outcome.detail} (the record holds no pull request)` }
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

/** The cycle's headline: its outcome once it has one, else its status. */
export const badge = (record: CycleRecord): { label: string; tone: Tone; reason: string } =>
  outcomeView(record) ??
  // Waiting on an E2E execution that runs is, to a reader, running.
  (record.status === 'waiting' && runningExecution(record)
    ? { label: 'Running', tone: 'accent', reason: '' }
    : { ...statusView(record.status), reason: '' })

// ── the four phases ──────────────────────────────────────────────────────

export const PHASES = ['Measure', 'Improve', 'Prove', 'Review'] as const
/** `off`: a step its settings never allowed, as opposed to one not reached yet. */
export type Mark = 'done' | 'now' | 'stop' | 'fail' | 'todo' | 'off'

/** The stage the cycle is at, or stopped at: a finished cycle's own stage is `done`. */
export function lastStage(record: CycleRecord): CycleStage {
  if (record.stage !== 'done') return record.stage
  return [...record.history].reverse().find((event) => event.stage !== 'done')?.stage ?? 'done'
}

/**
 * What the Review phase holds: the pull request, the wait to open it, or why
 * an accepted change got none. `null`: not reached.
 */
export function review(record: CycleRecord): { kind: 'opened' | 'waiting' | 'none'; detail: string } | null {
  const pr = record.pull_request
  if (pr) return { kind: 'opened', detail: `#${pr.number} ${pr.created ? 'opened' : 'found'}` }
  if (record.stage === 'pull_request' && record.status === 'waiting') {
    return { kind: 'waiting', detail: record.history.at(-1)?.detail ?? '' }
  }
  if (record.outcome?.code === 'decided' && record.decision?.outcome === 'accepted') {
    return { kind: 'none', detail: record.outcome.detail }
  }
  return null
}

/** Which phase the cycle reached, from what it recorded, so it never moves back. */
export function phaseOf(record: CycleRecord): number {
  const stage = lastStage(record)
  const stopped = stoppedFor(record)
  if (review(record)) return 3
  if (record.decision || stage === 'validation' || stopped === 'prove') return 2
  if (record.executions?.some((entry) => entry.role === 'candidate')) return 2
  if (record.proposal || record.candidate || stage === 'proposal' || stage === 'candidate' || stopped === 'improve') {
    return 1
  }
  return 0
}

/** One mark per phase: done before the one reached, its state on it, todo after. */
export function marks(record: CycleRecord): Mark[] {
  const at = phaseOf(record)
  const tone = badge(record).tone
  const current: Mark = isActive(record.status)
    ? 'now'
    : record.status === 'cancelled' || tone === 'warn'
      ? 'stop'
      : tone === 'alert'
        ? 'fail'
        : 'done'
  return PHASES.map((_, index) => (index < at ? 'done' : index === at ? current : 'todo'))
}

export type Group = 'you' | 'active' | 'done'

/**
 * Active; needing you, when there is something to do now (a setting to turn
 * on, a failure, a pull request not confirmed); else finished.
 */
export function groupOf(record: CycleRecord, current: LoopConfig | null | undefined): Group {
  if (isActive(record.status)) return 'active'
  const code = record.outcome?.code
  const label = badge(record).label
  const unconfirmed = code === 'proposed' && label !== OUTCOME.proposed.label
  return record.status === 'failed' || unconfirmed || fixFor(record, current) ? 'you' : 'done'
}

/** A settings section, in the order each needs the one before it. */
export type Setup = 'measure' | 'improve' | 'prove' | 'judge'

/** How far a new cycle can go with this configuration: 0 nothing runs, 3 it can open a pull request. */
export function readiness(config: LoopConfig | null | undefined): number {
  if (!config) return 0
  if (!config.candidate) return 1
  if (!config.validation) return 2
  return 3
}

export const READY_LINE = [
  'Not configured: a new cycle maps coverage and runs nothing until Measure is set up.',
  'New cycles measure and stop: no change is written until Improve is on.',
  'New cycles write a change but cannot prove it until Prove is on.',
  'New cycles open a pull request for an accepted change; a person reviews and merges it.',
]

const SETUP: Setup[] = ['measure', 'improve', 'prove']
/** The next section to set up, if any. */
export const nextSetup = (config: LoopConfig | null | undefined): Setup | null => SETUP[readiness(config)] ?? null

export const FIX: Record<Setup, { action: string; why: string }> = {
  measure: { action: 'Set up Measure', why: 'the loop was not configured' },
  judge: { action: 'Turn on the judge', why: 'no judge settles an ambiguous finding' },
  improve: { action: 'Turn on Improve', why: 'candidate work is off' },
  prove: { action: 'Turn on Prove', why: 'nothing can validate the candidate' },
}

const CONFIG_KEY: Record<string, Setup> = {
  candidate: 'improve',
  judge: 'judge',
  validation: 'prove',
}

/**
 * The setting a finished cycle stopped for: the outcome names the missing
 * `config.*` key.
 */
function stoppedFor(record: CycleRecord): Setup | null {
  if (isActive(record.status)) return null
  if (!record.config) return 'measure'
  const key = record.outcome?.detail.match(/config\.(candidate|judge|validation)\b/)?.[1]
  if (key) return CONFIG_KEY[key]
  return null
}

/** What to turn on for a cycle like this one to go further; `null` once the current settings have it. */
export function fixFor(record: CycleRecord, current: LoopConfig | null | undefined): Setup | null {
  const setup = stoppedFor(record)
  if (!setup) return null
  const done: Record<Setup, unknown> = {
    measure: current,
    judge: current?.judge,
    improve: current?.candidate,
    prove: current?.validation,
  }
  return done[setup] ? null : setup
}

const DOING: Record<CycleStage, string> = {
  coverage: 'Finding the E2E tests that cover the goal',
  baseline: "Measuring today's Harness on the E2E",
  awaiting_execution: 'Waiting for the E2E execution',
  analysis: "Reading the execution's reports",
  triage: 'Deciding whether the findings are worth a change',
  proposal: 'Proposing one change',
  candidate: 'Writing the candidate change',
  validation: 'Running the candidate on the E2E',
  pull_request: 'Opening the pull request',
  done: 'Finishing',
}

const RUNNING: Record<ExecutionRole, string> = {
  source: 'Waiting for the execution to finish',
  baseline: "Measuring today's Harness on the E2E",
  candidate: 'Running the candidate on the E2E',
}

/** The E2E execution the cycle waits on now, if any. */
export const runningExecution = (record: CycleRecord) =>
  isActive(record.status)
    ? record.executions?.filter((entry) => entry.state === 'running' || entry.state === 'dispatching').at(-1)
    : undefined

/** What is happening now, or why it stopped and what would let it go on. */
export function nowView(
  record: CycleRecord,
  current?: LoopConfig | null,
): { tone: Tone; headline: string; detail: string; fix: Setup | null } {
  if (isActive(record.status)) {
    const running = runningExecution(record)
    const headline =
      record.status === 'queued'
        ? 'Queued'
        : running
          ? RUNNING[running.role]
          : DOING[record.stage]
    return {
      tone: 'accent',
      headline,
      detail: record.next?.detail ?? record.history.at(-1)?.detail ?? '',
      fix: null,
    }
  }
  const view = badge(record)
  const fix = fixFor(record, current)
  // A short reason reads as the headline itself; a long one stays the detail.
  const short = !fix && view.reason.length <= SHORT_REASON
  return {
    tone: view.tone,
    headline: fix
      ? `${view.label}: ${FIX[fix].why}`
      : short && view.reason
        ? `${view.label}: ${view.reason}`
        : view.label,
    detail: short ? '' : view.reason,
    fix,
  }
}

const SHORT_REASON = 90

/** An execution id short enough for a line: `plan-e46e62e9`. */
export const shortId = (id: string) => (id.length > 16 ? id.slice(0, id.startsWith('plan-') ? 13 : 8) : id)

/** The row's second line after its source: what it does, why it stopped or how it ended. */
export function rowLine(record: CycleRecord, current?: LoopConfig | null): string {
  if (isActive(record.status)) return `${PHASES[phaseOf(record)]} · ${nowView(record).headline}`
  const fix = fixFor(record, current)
  return fix ? FIX[fix].why : badge(record).reason
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
    const subject = `${analysis.role} · ${shortId(analysis.execution_id)}`
    for (const reason of analysis.unknowns) found.push({ kind: 'unknown', subject, reason })
    if (analysis.sentinel?.error) {
      found.push({ kind: 'unknown', subject: `${subject} · Sentinel`, reason: analysis.sentinel.error })
    }
    // The reports' own unknowns may already say it.
    if (
      !analysis.child_transcripts_complete &&
      !analysis.unknowns.some((reason) => reason.startsWith('child transcripts'))
    ) {
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
  function_call_errors: 'function call errors',
}

const trim = (value: number) => (Number.isInteger(value) ? String(value) : value.toFixed(2))

/** A mean or delta in the metric's own unit: a fraction for rates, ms for wall time. */
export function formatMetric(metric: CriterionMetric, value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value)) return '—'
  switch (metric) {
    case 'pass_rate':
    case 'criterion_score':
      return `${Number((value * 100).toFixed(1))}%`
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

/** The criterion's own name, or its metric's. */
export const criterionName = (criterion: Criterion) =>
  criterion.criterion_id ?? (criterion.pattern ? `${criterion.pattern} per run` : METRIC[criterion.metric])

/** A run's value of the criterion, as the decision reads it; undefined when unmeasured. */
export function runValue(run: EvidenceRun, criterion: Criterion): number | undefined {
  switch (criterion.metric) {
    case 'pass_rate':
      return run.completed && run.status === 'passed' ? 1 : 0
    case 'criterion_score':
      return run.criteria?.[criterion.criterion_id ?? '']
    case 'signal_per_run':
      return run.signals?.[criterion.pattern ?? '']
    case 'cost_usd':
      return run.cost_usd
    case 'duration':
      return run.wall_time_ms
    case 'tokens':
      return run.total_tokens
    case 'function_calls':
      return run.function_calls
    case 'function_call_errors':
      return run.function_call_errors
  }
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
  // Rates move in points: a relative change of a rate only confuses.
  if (metric === 'pass_rate' || metric === 'criterion_score') {
    return `${sign}${Number((Math.abs(change.absolute) * 100).toFixed(1))} pp`
  }
  const relative = change.relative === null ? '' : `, ${sign}${Math.abs(change.relative * 100).toFixed(1)}%`
  return `${sign}${formatMetric(metric, Math.abs(change.absolute))}${relative}`
}

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
      !validation.stack_name.trim()
    ) {
      return 'validation needs a stack_name and a source template with {commit}'
    }
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

/** `38m`, `2h`, `1h 12m`, `45s`: a budget's time in the words a person uses. */
export function span(ms: number): string {
  const minutes = Math.round(ms / 60_000)
  if (minutes < 1) return `${Math.round(ms / 1000)}s`
  const hours = Math.floor(minutes / 60)
  return hours ? `${hours}h${minutes % 60 ? ` ${minutes % 60}m` : ''}` : `${minutes}m`
}

/** `1 case`, `5 cases`. */
export const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? '' : 's'}`

// ── what an ended cycle found, and what can follow ──────────────────────

/** One criterion the runs lost, over every execution the cycle read. */
export interface Lost {
  scenario: string
  criterion: string
  /** The question the test's contract asks of it, when the cycle froze one. */
  description?: string
  /** Runs that lost points on it, of `runs` read runs of its scenario. */
  lost: number
  runs: number
  /** The evaluator's `name=false` checks: what the runs did not do. */
  failed: string[]
  /** Each losing run's evaluator detail, in order. */
  details: string[]
}

/** The first pass writes `run <scenario>/<run>: criterion <id> awarded <n> of <m>: <why>`. */
const AWARD = /^run ([^/\s]+)\/\S+: criterion (\S+) awarded [^:]*:? ?(.*)$/

/** The lost criteria, one entry each, most often lost first. */
export function lostCriteria(record: CycleRecord): Lost[] {
  const analyses = record.analyses ?? []
  const tests = record.coverage?.selected ?? record.plan?.tests ?? []
  const found = new Map<string, Lost>()
  for (const finding of analyses.flatMap((analysis) => analysis.findings)) {
    if (finding.class !== 'defect' && finding.class !== 'improvement_opportunity') continue
    const match = finding.observation.match(AWARD)
    if (!match) continue
    const [, scenario, criterion, why] = match
    const key = `${scenario}:${criterion}`
    const entry = found.get(key) ?? {
      scenario,
      criterion,
      description: tests.find((test) => test.scenario_id === scenario)?.criteria.find((item) => item.id === criterion)
        ?.description,
      lost: 0,
      runs: 0,
      failed: [],
      details: [],
    }
    entry.lost += 1
    entry.details.push(why)
    for (const [, name] of why.matchAll(/(\w+)=false\b/g)) if (!entry.failed.includes(name)) entry.failed.push(name)
    found.set(key, entry)
  }
  for (const entry of found.values()) {
    const read = analyses
      .flatMap((analysis) => analysis.scenarios ?? [])
      .filter((scenario) => scenario.scenario_id === entry.scenario)
      .reduce((sum, scenario) => sum + scenario.runs, 0)
    entry.runs = Math.max(read, entry.lost)
  }
  return [...found.values()].sort((a, b) => b.lost - a.lost)
}

/** An ended cycle that changed nothing can measure its finding again. */
export function canRetry(record: CycleRecord): boolean {
  if (isActive(record.status) || record.pull_request || record.decision?.outcome === 'accepted') return false
  if (record.outcome?.code === 'no_coverage') return false
  return 'objective' in record.input || (record.analyses ?? []).some((analysis) => analysis.role === 'source')
}

/** An ended cycle's result as a sentence. */
export function resultHeadline(record: CycleRecord, lost: number): string {
  if (record.status === 'cancelled') return 'The cycle was cancelled before it decided anything.'
  if (record.status === 'failed') return 'The cycle failed before it decided anything.'
  switch (record.outcome?.code) {
    case 'no_action':
      return lost
        ? 'The runs lost points, but the cycle stopped before proposing a change.'
        : 'Nothing in the runs was worth changing.'
    case 'no_coverage':
      return 'No E2E test measures this, so nothing ran.'
    case 'budget':
      return 'The cycle reached its budget before it could decide.'
    case 'inconclusive':
      return 'The cycle ended without a verdict.'
    case 'proposed':
      return 'The change was proven and a pull request was opened.'
    default:
      return record.decision ? `The change was ${record.decision.outcome}.` : 'The cycle ended.'
  }
}
