// Pure presentation logic for the session monitor: labels, tones, the
// pipeline, filters and aggregates. No React, no host — unit-tested.
import type {
  AnalysisRecord,
  AnalysisStatus,
  Diagnostic,
  EntryRef,
  Failure,
  JsonValue,
  MonitorUsage,
  SignalAssessment,
  Snapshot,
  Suggestion,
} from './types'

export type Tone = 'ok' | 'accent' | 'warn' | 'alert' | 'neutral'

export const ACTIVE: AnalysisStatus[] = ['queued', 'collecting', 'judging', 'investigating']

export function isActive(status: AnalysisStatus): boolean {
  return ACTIVE.includes(status)
}

export const STAGE_LABEL: Record<AnalysisStatus, string> = {
  queued: 'Queued',
  collecting: 'Collecting',
  judging: 'Triage',
  investigating: 'Investigating',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

const FAILURE_LABEL: Record<string, string> = {
  external_outcome_unknown: 'Failed · outcome unknown',
  source_advanced: 'Failed · session changed',
  inconsistent_evidence: 'Failed · session changed',
  deadline: 'Failed · deadline',
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`
}

/** The status pill: what the monitor did, never a verdict on the task. */
export function statusPresentation(record: AnalysisRecord): { label: string; tone: Tone } {
  const { status, failure, counters, pending_reason } = record
  switch (status) {
    case 'queued':
      return { label: 'Queued', tone: 'accent' }
    case 'collecting':
      return pending_reason ? { label: 'Collecting · waiting', tone: 'warn' } : { label: 'Collecting', tone: 'accent' }
    case 'judging':
      return { label: 'Triage', tone: 'accent' }
    case 'investigating':
      return { label: 'Investigating', tone: 'accent' }
    case 'completed':
      return counters.suggestions > 0
        ? { label: plural(counters.suggestions, 'suggestion'), tone: 'ok' }
        : { label: 'No suggestions', tone: 'ok' }
    case 'cancelled':
      return { label: 'Cancelled', tone: 'neutral' }
    case 'failed':
      if (failure?.code === 'coverage_insufficient') {
        return { label: 'Insufficient evidence', tone: 'warn' }
      }
      return {
        label: (failure && FAILURE_LABEL[failure.code]) ?? `Failed at ${stageName(failure?.stage)}`,
        tone: 'alert',
      }
  }
}

function stageName(stage: AnalysisStatus | undefined): string {
  switch (stage) {
    case 'judging':
      return 'triage'
    case 'investigating':
      return 'investigation'
    case 'collecting':
    case 'queued':
      return 'collection'
    default:
      return 'an unknown stage'
  }
}

/** The quiet second line of a history row. */
export function rowDetail(record: AnalysisRecord): string {
  if (record.status === 'collecting' && record.pending_reason) {
    return record.pending_reason.includes('descendant') ? 'children running' : 'waiting'
  }
  if (record.status === 'failed' && record.failure) return failureShort(record.failure)
  if (record.status === 'cancelled') {
    const at = [...record.stages].reverse().find((stage) => stage.status !== 'cancelled')
    return at ? `during ${stageName(at.status)}` : 'cancelled'
  }
  if (record.counters.diagnostics > 0) return plural(record.counters.diagnostics, 'signal')
  return record.status === 'completed' ? 'no signals' : STAGE_LABEL[record.status].toLowerCase()
}

function failureShort(failure: Failure): string {
  if (failure.code.startsWith('judge_')) return failure.code.slice('judge_'.length).replaceAll('_', ' ')
  if (failure.code === 'coverage_insufficient') return 'over the evidence limit'
  return failure.code.replaceAll('_', ' ')
}

export type Filter = 'all' | 'active' | 'suggestions' | 'failed'

export function matchesFilter(record: AnalysisRecord, filter: Filter): boolean {
  switch (filter) {
    case 'all':
      return true
    case 'active':
      return isActive(record.status)
    case 'suggestions':
      return record.status === 'completed' && record.counters.suggestions > 0
    case 'failed':
      return record.status === 'failed'
  }
}

export type StepState = 'done' | 'running' | 'waiting' | 'failed' | 'cancelled' | 'skipped' | 'pending'

export interface PipelineStep {
  key: 'queued' | 'collecting' | 'judging' | 'investigating' | 'done'
  label: string
  state: StepState
  /** Time spent in the step, when it started and ended. */
  durationMs?: number
}

const STEPS: Array<Pick<PipelineStep, 'key' | 'label'>> = [
  { key: 'queued', label: 'Queued' },
  { key: 'collecting', label: 'Collecting' },
  { key: 'judging', label: 'Triage' },
  { key: 'investigating', label: 'Investigating' },
  { key: 'done', label: 'Completed' },
]

/**
 * The five steps with their state and duration, from the recorded stage
 * start times. An investigation the routing skipped is `skipped`.
 */
export function pipeline(record: AnalysisRecord, now: number): PipelineStep[] {
  const started = new Map<string, number>()
  for (const stage of record.stages) {
    if (!started.has(stage.status)) started.set(stage.status, stage.at)
  }
  const endOf = (index: number): number | undefined => {
    for (let next = index + 1; next < STEPS.length; next += 1) {
      const key = STEPS[next].key === 'done' ? undefined : started.get(STEPS[next].key)
      if (key !== undefined) return key
    }
    return record.completed_at
  }
  const failedAt = record.status === 'failed' ? (record.failure?.stage ?? 'queued') : undefined
  const cancelledAt =
    record.status === 'cancelled'
      ? ([...record.stages].reverse().find((stage) => stage.status !== 'cancelled')?.status ?? 'queued')
      : undefined
  const stopIndex = STEPS.findIndex((step) => step.key === (failedAt ?? cancelledAt ?? record.status))
  return STEPS.map((step, index) => {
    if (step.key === 'done') {
      const state: StepState =
        record.status === 'completed' ? 'done' : record.status === 'failed' ? 'failed' : 'pending'
      return {
        ...step,
        label: record.status === 'failed' ? 'Failed' : record.status === 'cancelled' ? 'Cancelled' : 'Completed',
        state: record.status === 'cancelled' ? 'cancelled' : state,
        durationMs: record.completed_at ? record.completed_at - record.created_at : undefined,
      }
    }
    const start = started.get(step.key)
    const end = endOf(index)
    let state: StepState
    if (failedAt === step.key) state = 'failed'
    else if (cancelledAt === step.key) state = 'cancelled'
    else if (start !== undefined && (end !== undefined || index < stopIndex)) state = 'done'
    else if (record.status === step.key) {
      state = step.key === 'collecting' && record.pending_reason ? 'waiting' : 'running'
    } else if (
      step.key === 'investigating' &&
      record.status === 'completed' &&
      record.routing &&
      !record.routing.investigate
    ) {
      state = 'skipped'
    } else state = 'pending'
    const durationMs =
      start !== undefined ? (end ?? (state === 'running' || state === 'waiting' ? now : undefined)) : undefined
    return {
      ...step,
      state,
      durationMs: start !== undefined && durationMs !== undefined ? durationMs - start : undefined,
    }
  })
}

/** Remaining budget, or `null` once terminal. */
export function remainingMs(record: AnalysisRecord, now: number): number | null {
  return isActive(record.status) ? Math.max(0, record.deadline - now) : null
}

export interface MonitorSummary {
  running: number
  pending: number
  today: number
  /** Known LLM cost of today's analyses; Jev cost is never reported. */
  todayCostUsd: number
  /** Today's analyses whose cost is unknown. */
  todayCostUnknown: number
}

export function monitorSummary(records: AnalysisRecord[], now: number): MonitorSummary {
  const midnight = new Date(now)
  midnight.setHours(0, 0, 0, 0)
  const today = records.filter((record) => record.created_at >= midnight.getTime())
  return {
    running: records.filter((record) => ['collecting', 'judging', 'investigating'].includes(record.status)).length,
    pending: records.filter((record) => record.status === 'queued').length,
    today: today.length,
    todayCostUsd: today.reduce((sum, record) => sum + (record.usage.llm_cost_usd ?? 0), 0),
    todayCostUnknown: today.filter(
      (record) => record.usage.llm_cost_usd === undefined && record.usage.llm_input_tokens !== undefined,
    ).length,
  }
}

export function totalTokens(usage: MonitorUsage): { judge: number; llm?: number } {
  const llm =
    usage.llm_input_tokens === undefined && usage.llm_output_tokens === undefined
      ? undefined
      : (usage.llm_input_tokens ?? 0) + (usage.llm_output_tokens ?? 0)
  return { judge: usage.judge_input_tokens + usage.judge_output_tokens, llm }
}

/** `61.3k`, `1,204`, `—` for unknown. */
export function formatTokens(tokens: number | undefined): string {
  if (tokens === undefined || !Number.isFinite(tokens)) return '—'
  if (tokens >= 10_000) return `${(tokens / 1000).toFixed(1)}k`
  return new Intl.NumberFormat('en-US').format(tokens)
}

/** Unknown cost stays unknown, never `$0`. */
export function formatCost(usd: number | undefined | null): string {
  if (usd === undefined || usd === null || !Number.isFinite(usd)) return 'not reported'
  return usd < 1 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`
}

export function shortHash(value: string | undefined): string {
  if (!value) return '—'
  const bare = value.includes(':') ? value.slice(value.indexOf(':') + 1) : value
  return bare.length > 10 ? `${bare.slice(0, 4)}…${bare.slice(-3)}` : bare
}

/** The preview entry a reference points to, if it was shown to the models. */
export function previewEntry(snapshot: Snapshot | undefined, ref: EntryRef): Record<string, JsonValue> | undefined {
  const session = snapshot?.sessions.find((candidate) => candidate.session_id === ref.session_id)
  const entry = session?.preview.find(
    (item) => typeof item === 'object' && item !== null && !Array.isArray(item) && item.entry_id === ref.entry_id,
  )
  return entry as Record<string, JsonValue> | undefined
}

/** A short chip label for an evidence reference: `c2 · functions::info`. */
export function entryLabel(snapshot: Snapshot | undefined, ref: EntryRef): string {
  const entry = previewEntry(snapshot, ref)
  const message = entry?.message as Record<string, JsonValue> | undefined
  const custom = entry?.custom as Record<string, JsonValue> | undefined
  const tail = ref.entry_id.replace(/^e_t_[0-9a-z]+_/i, '')
  const id = tail.length > 14 ? `${tail.slice(0, 12)}…` : tail
  const kind =
    (typeof message?.function_id === 'string' && message.function_id.split('::').slice(-2).join('::')) ||
    (typeof custom?.custom_type === 'string' && custom.custom_type) ||
    (typeof message?.role === 'string' && message.role) ||
    ''
  return kind ? `${id} · ${kind}` : id
}

export function assessmentFor(
  diagnostic: Diagnostic,
  assessments: SignalAssessment[] | undefined,
): SignalAssessment | undefined {
  return assessments?.find((assessment) => assessment.fingerprint === diagnostic.fingerprint)
}

/** Suggestions (1-based) that cite at least one of the signal's entries. */
export function suggestionsUsing(diagnostic: Diagnostic, suggestions: Array<{ evidence: EntryRef[] }>): number[] {
  const refs = new Set(diagnostic.evidence.map((ref) => `${ref.session_id}\u0000${ref.entry_id}`))
  return suggestions
    .map((suggestion, index) =>
      suggestion.evidence.some((ref) => refs.has(`${ref.session_id}\u0000${ref.entry_id}`)) ? index + 1 : 0,
    )
    .filter((index) => index > 0)
}

/** The E2E plan as Markdown, for "Copy E2E plan". */
export function planMarkdown(
  title: string,
  plan: {
    scenario_id: string | null
    reproduction: string
    invariants: string[]
    primary_metric: string
    expectation: string
    non_regression_controls: string[]
  },
): string {
  return [
    `## E2E plan · ${title}`,
    `Scenario: ${plan.scenario_id ?? 'new case needed'}`,
    '',
    `Reproduction: ${plan.reproduction}`,
    '',
    'Invariants:',
    ...plan.invariants.map((item) => `- ${item}`),
    '',
    `Primary metric: ${plan.primary_metric}`,
    `Expectation: ${plan.expectation}`,
    '',
    'Non-regression controls:',
    ...plan.non_regression_controls.map((item) => `- ${item}`),
  ].join('\n')
}

/** `03 Oct 10:12`, local time: when something was decided. */
export function formatStamp(ms: number): string {
  const date = new Date(ms)
  const month = date.toLocaleString('en-US', { month: 'short' })
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${pad(date.getDate())} ${month} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export interface BriefInput {
  analysisId: string
  sessionId: string
  turnId: string
  harnessVersion?: string
  /** 0-based. */
  index: number
  suggestion: Suggestion
  /** Where the suggestion stands, as a sentence (`Accepted by layon, 02 Oct 21:14`). */
  status?: string
  /** The registered success criterion, as a sentence. */
  criterion?: string
  /** What the replay at the decision point showed, as a sentence. */
  replay?: string
  /** The directory the investigation read code in. */
  codeRoot?: string
}

/**
 * The handoff to whoever implements a suggestion, from stored fields only: a
 * person or a chat reads it, a model never summarizes it. A section with
 * nothing to say is left out.
 */
export function briefMarkdown(input: BriefInput): string {
  const { suggestion, index } = input
  const { validation } = suggestion
  const section = (title: string, lines: string[]): string[] => {
    const kept = lines.filter((line) => line.trim() !== '')
    return kept.length > 0 ? ['', `## ${title}`, ...kept] : []
  }
  const codeRefs = suggestion.code_refs.map((ref) =>
    ref.line_from === ref.line_to ? `- ${ref.path}:${ref.line_from}` : `- ${ref.path}:${ref.line_from}-${ref.line_to}`,
  )
  const entries = suggestion.evidence.map((ref) => ref.entry_id)
  const controls = [...validation.invariants, ...validation.non_regression_controls]
  return [
    `# Implement: ${suggestion.title}`,
    [
      `Analysis ${input.analysisId}`,
      `S${index + 1}`,
      `observed session ${input.sessionId}`,
      `turn ${input.turnId}`,
      input.harnessVersion ? `Harness ${input.harnessVersion}` : '',
    ]
      .filter(Boolean)
      .join(' · '),
    ...(input.status ? [`Status: ${input.status}`] : []),
    ...section('Observation', [suggestion.observation]),
    ...section('Hypothesis (not proven)', [suggestion.hypothesis]),
    ...section('Proposed change', [`Harness area: ${suggestion.harness_component}`, suggestion.proposed_change]),
    ...section('Expected effect', [suggestion.expected_effect]),
    ...section('Code read', codeRefs.length > 0 && input.codeRoot ? [`In ${input.codeRoot}`, ...codeRefs] : codeRefs),
    ...section(
      'Evidence in the analysis',
      entries.length > 0 ? [`${entries.join(', ')} (open ${input.analysisId} for the entries)`] : [],
    ),
    ...section('Limitations', [suggestion.limitations]),
    ...section('Replay at the decision point', input.replay ? [input.replay] : []),
    ...section('Non-regression in E2E', [
      `Scenario: ${validation.scenario_id ?? 'none named'}`,
      ...(input.criterion ? [`Criterion: ${input.criterion}`] : []),
      `Primary metric: ${validation.primary_metric}`,
      `Expectation: ${validation.expectation}`,
      ...(controls.length > 0 ? ['Controls:', ...controls.map((control) => `- ${control}`)] : []),
    ]),
  ].join('\n')
}
