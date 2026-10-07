// Pure logic of the Monitor shell: the status card, the queue and cost lines,
// the history rows, the filter copy, notices and how a palette or command
// context reaches the page. No React, no host — unit-tested.
import { formatBytes } from '@iii-dev/console-ui/format'
import {
  type Filter,
  isActive,
  matchesFilter,
  monitorSummary,
  rowDetail,
  statusPresentation,
  type Tone,
} from '../../model'
import type {
  AnalysisRecord,
  CapacityRejection,
  MonitorConfig,
  MonitorCost,
  MonitorLimits,
  MonitorState,
  ReviewSummary,
  ReviewsResponse,
  SuggestionReview,
  TriageAvailability,
} from '../../types'
import { failureCode, failureHeadline } from './detail/notices'
import { formatCostShort, investigationCaps, parseSizes, plural, shortId } from './detail/present'
import { MIN_ESTIMATE, modelWithLevel, tokenCap, triageHint } from './settings-model'
import { ago, clock, dayClock, span, spanRange } from './time'

export { clock }

/** `eval::list` returns at most this many rows (`limit` in api.ts). */
export const LIST_LIMIT = 200
/** Refresh cadence while something is running or the observer is not bound yet. */
export const POLL_MS = 3_000
/** A capacity rejection stops being worth a notice after an hour. */
export const REJECTION_WINDOW_MS = 3_600_000
/** Wide enough for the model id and the queue line without wrapping. */
export const SIDEBAR_WIDTH = 340
/** The sidebar, the hairline, and a detail that stays readable (560). */
export const NARROW_BELOW = SIDEBAR_WIDTH + 1 + 560

export const FILTERS: Array<{ value: Filter; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'active', label: 'Active' },
  { value: 'suggestions', label: 'Suggestions' },
  { value: 'failed', label: 'Failed' },
]

export function emptyFilterTitle(filter: Filter, reviewing = false): string {
  switch (filter) {
    case 'active':
      return 'No active analyses'
    case 'suggestions':
      return reviewing ? 'Nothing to review' : 'No analyses with suggestions'
    case 'failed':
      return 'No failed analyses'
    case 'all':
      return 'No analyses yet'
  }
}

export type CardState = 'loading' | 'unconfigured' | 'unavailable' | 'paused' | 'capped' | 'observing'

/**
 * What the status card says. A failed read of the monitor is unavailable even
 * when an older answer is still held. An observer that did not bind only
 * matters while observation is meant to be on, so a paused or unconfigured
 * monitor keeps saying so (the notice below the card still reports the
 * binding).
 */
export function cardState(state: MonitorState | null, failed = false): CardState {
  if (failed) return 'unavailable'
  if (!state) return 'loading'
  if (!state.config) return 'unconfigured'
  if (!state.config.enabled) return 'paused'
  if (!state.observer_bound) return 'unavailable'
  return state.cost.capped ? 'capped' : 'observing'
}

export const CARD_LABEL: Record<Exclude<CardState, 'loading'>, string> = {
  unconfigured: 'Not configured',
  unavailable: 'Unavailable',
  paused: 'Paused',
  capped: 'Paused for today',
  observing: 'Observing',
}

/** `2 running · 1 pending · cap 500`; the cap (`max_active_analyses`) is left out until the state is read. */
export function queueLine(records: AnalysisRecord[], now: number, cap: number | undefined): string {
  const { running, pending } = monitorSummary(records, now)
  return `${running} running · ${pending} pending${cap === undefined ? '' : ` · cap ${cap}`}`
}

/**
 * `14 analyses · $0.91 monitor · Jev not included`. A cost the API did not
 * report stays unknown ("cost not reported"), never `$0`; Jev's cost is never
 * part of the sum, and the line says so once Jev was called.
 */
export function todayLine(records: AnalysisRecord[], now: number): string {
  const summary = monitorSummary(records, now)
  const count = `${summary.today} ${summary.today === 1 ? 'analysis' : 'analyses'}`
  const midnight = new Date(now)
  midnight.setHours(0, 0, 0, 0)
  const today = records.filter((record) => record.created_at >= midnight.getTime())
  const known = today.filter((record) => record.usage.llm_cost_usd !== undefined).length
  const jev = today.some((record) => record.usage.judge_calls > 0) ? ' · Jev not included' : ''
  if (summary.todayCostUnknown > 0) {
    return known === 0
      ? `${count} · cost not reported`
      : `${count} · ${formatCostShort(summary.todayCostUsd)} monitor · ${summary.todayCostUnknown} not reported${jev}`
  }
  return known === 0 ? count : `${count} · ${formatCostShort(summary.todayCostUsd)} monitor${jev}`
}

/** The paused notice: what stops, and what keeps going. */
export function pausedDetail(running: number, pending: number): string {
  const continues = (count: number, kind: string) =>
    `${count} ${kind} ${count === 1 ? 'analysis continues' : 'analyses continue'}.`
  return [
    "New sessions aren't analyzed.",
    running > 0 ? continues(running, 'running') : pending > 0 ? continues(pending, 'queued') : '',
    'Sessions finished while paused can be analyzed by ID.',
  ]
    .filter(Boolean)
    .join(' ')
}

/** The last capacity rejection, while it is recent enough to matter. */
export function recentRejection(state: MonitorState | null, now: number): CapacityRejection | undefined {
  const rejection = state?.last_rejection
  return rejection && now - rejection.at < REJECTION_WINDOW_MS ? rejection : undefined
}

/**
 * The last turned-away session, and whether the monitor is still full. The
 * backend never clears `last_rejection`, so "at capacity" is only a present
 * fact while the unfinished analyses still fill the cap; afterwards it is a
 * past event, said with its time.
 */
export interface CapacityNotice {
  sessionId: string
  at: number
  unfinished: number
  live: boolean
}

export function capacityNotice(
  state: MonitorState | null,
  records: AnalysisRecord[] | null,
  now: number,
): CapacityNotice | undefined {
  const rejection = recentRejection(state, now)
  // A session skipped by the cost cap is the status card's "Paused for today", not a full queue.
  if (!state || !rejection || rejection.reason === 'cost_cap') return undefined
  const { running, pending } = monitorSummary(records ?? [], now)
  const unfinished = running + pending
  return {
    sessionId: rejection.session_id,
    at: rejection.at,
    unfinished,
    live: unfinished >= state.limits.max_active_analyses,
  }
}

/**
 * Poll while an analysis is unfinished or the observer is not bound yet (it
 * binds a moment after the worker starts); events and focus cover the rest.
 */
export function needsPolling(records: AnalysisRecord[] | null, state: MonitorState | null = null): boolean {
  return state?.observer_bound === false || Boolean(records?.some((record) => isActive(record.status)))
}

/**
 * The quiet second line of a history row: the model's own wording, except
 * that an over-size capture says which limit it went over.
 */
export function rowMeta(record: AnalysisRecord): string {
  if (record.failure?.code === 'coverage_insufficient') {
    const sizes = parseSizes(record.failure.message)
    if (sizes) return `over ${formatBytes(sizes.limit).replace('.0 ', ' ')}`
  }
  return rowDetail(record)
}

/** `eval::analyze` refuses a turn whose analysis was deleted unless `reanalyze` is set. */
export function isDeletedAnalysis(message: string): boolean {
  return /was deleted; set reanalyze/.test(message)
}

export interface RowStatus {
  label: string
  tone: Tone
  /** A finished analysis with nothing to act on reads quieter than one with suggestions. */
  quiet: boolean
}

export function rowStatus(record: AnalysisRecord): RowStatus {
  const { label, tone } = statusPresentation(record)
  return { label, tone, quiet: tone === 'ok' && record.counters.suggestions === 0 }
}

export function rowTitle(record: AnalysisRecord): string {
  return record.source_title?.trim() || record.session_id
}

/** What a palette row or command asks the page to do. */
export type OpenRequest = { seq: number } & ({ type: 'analysis'; evaluationId: string } | { type: 'analyze' })

/** Reads `PageRenderProps.panelContext`; anything else is ignored. */
export function parseOpenContext(seq: number, context: unknown): OpenRequest | null {
  if (typeof context !== 'object' || context === null) return null
  const { type, evaluationId } = context as { type?: unknown; evaluationId?: unknown }
  if (type === 'analysis' && typeof evaluationId === 'string' && evaluationId) {
    return { seq, type, evaluationId }
  }
  if (type === 'analyze') return { seq, type }
  return null
}

// ── The daily cost cap ─────────────────────────────────────────────────

const DAY_MS = 24 * 60 * 60 * 1000

/** Under "Paused for today": why nothing new starts, and what still does. */
export function cappedNote(cost: MonitorCost): string {
  return `Cost cap ${formatCostShort(cost.cap_usd)} reached (${formatCostShort(cost.today_usd)} reported). Manual analyses still run. New ones resume at ${clock(cost.since + DAY_MS)}.`
}

/** The status card's Cap row: `$0.91 of $5.00 · 2 unknown`; `null` while no cap is set. */
export function capRow(cost: MonitorCost): string | null {
  if (cost.cap_usd === undefined) return null
  const unknown = cost.today_unknown > 0 ? ` · ${cost.today_unknown} unknown` : ''
  return `${formatCostShort(cost.today_usd)} of ${formatCostShort(cost.cap_usd)}${unknown}`
}

/**
 * The Skipped row while the cap holds: the monitor only keeps its last turned-away session, so it names that one
 * instead of counting.
 */
export function skippedRow(state: MonitorState | null): string | null {
  const rejection = state?.last_rejection
  if (!state?.cost.capped || rejection?.reason !== 'cost_cap' || rejection.at < state.cost.since) return null
  return `last ${clock(rejection.at)} · cost cap`
}

// ── Whether triage can run ─────────────────────────────────────────────

/** A triage failure stays the monitor's last word about Jev for this long. */
export const TRIAGE_FAILURE_WINDOW_MS = 3_600_000
/** The code of Jev's HTTP 402: the provider check lists models, so it cannot see credits. */
export const OUT_OF_CREDITS = 'out_of_credits'

/** Why Jev cannot triage, and how the monitor knows. */
export interface TriageProblem {
  /** `out_of_credits`, or the code of the provider check (`missing_key`, `provider_unavailable`, `unreachable`). */
  code: string
  at: number
  /** `analysis`: an analysis hit it just now; only a new analysis tells whether it is fixed. `check`: the provider check said so. */
  source: 'check' | 'analysis'
}

/**
 * The provider check, else the newest triage outcome among the analyses: an HTTP 402 within the hour, until a later
 * analysis got an answer (its routing is set). `undefined` is no problem known.
 */
export function triageProblem(
  triage: TriageAvailability | undefined,
  records: readonly AnalysisRecord[] | null,
  now: number,
): TriageProblem | undefined {
  if (triage && !triage.available) return { code: triage.code ?? 'unreachable', at: triage.checked_at, source: 'check' }
  let newest: { at: number; credits: boolean } | undefined
  for (const record of records ?? []) {
    const credits = record.failure !== undefined && failureCode(record.failure) === 'judge_out_of_credits'
    if (!credits && !record.routing) continue
    const at = record.completed_at ?? record.updated_at
    if (!newest || at > newest.at) newest = { at, credits }
  }
  return newest?.credits && now - newest.at < TRIAGE_FAILURE_WINDOW_MS
    ? { code: OUT_OF_CREDITS, at: newest.at, source: 'analysis' }
    : undefined
}

function reasonWords(code: string): string {
  switch (code) {
    case OUT_OF_CREDITS:
      return 'out of credits (HTTP 402)'
    case 'missing_key':
      return 'no API key'
    default:
      return code.replaceAll('_', ' ')
  }
}

/** The Triage row of the status card. */
export function triageRow(problem: TriageProblem | undefined, checked: boolean, provider: string): string {
  if (problem)
    return `unavailable · ${problem.code === OUT_OF_CREDITS ? 'HTTP 402' : problem.code.replaceAll('_', ' ')}`
  return checked ? `${provider} · available` : provider
}

/** The warning panel of the status card. */
export function triageNotice(problem: TriageProblem): string {
  const when = problem.source === 'analysis' ? 'last triage' : 'checked'
  return `Jev: ${reasonWords(problem.code)}, ${when} ${clock(problem.at)}. New analyses will fail at triage.`
}

// ── What an analysis costs ─────────────────────────────────────────────

/** `count` analyses with a reported cost; the ranges exist once there are `MIN_ESTIMATE` of them. */
export interface Estimate {
  count: number
  cost?: [low: number, high: number]
  time?: [low: number, high: number]
}

function middle(sorted: number[]): number {
  const half = Math.floor(sorted.length / 2)
  return sorted.length % 2 === 1 ? sorted[half] : (sorted[half - 1] + sorted[half]) / 2
}

/**
 * From the monitor's own history, never a forecast: from the median to the most an analysis cost so far, and the
 * same for the time. The cost comes from the monitor (every stored analysis); the time from the listed ones.
 */
export function estimateFor(state: MonitorState | null, records: readonly AnalysisRecord[] | null): Estimate {
  const stats = state?.cost.per_analysis
  const config = state?.config
  if (!stats || !config) return { count: 0 }
  if (stats.count < MIN_ESTIMATE || stats.median === undefined || stats.max === undefined) {
    return { count: stats.count }
  }
  const spans = (records ?? [])
    .filter(
      (record) =>
        record.status === 'completed' &&
        record.analyst !== undefined &&
        record.completed_at !== undefined &&
        record.model.model === config.model.model &&
        record.model.provider === config.model.provider &&
        Boolean(record.code_root) === Boolean(config.code_repository),
    )
    .map((record) => (record.completed_at ?? record.created_at) - record.created_at)
    .sort((a, b) => a - b)
  return {
    count: stats.count,
    cost: [stats.median, stats.max],
    time: spans.length >= MIN_ESTIMATE ? [middle(spans), spans[spans.length - 1]] : undefined,
  }
}

/** `≈ $0.38–1.27, 2–6 min`; `null` while there is no estimate. */
export function estimateText(estimate: Estimate): string | null {
  if (!estimate.cost) return null
  const [low, high] = estimate.cost
  const cost = low === high ? formatCostShort(low) : `${formatCostShort(low)}–${formatCostShort(high).slice(1)}`
  return `≈ ${[cost, estimate.time ? spanRange(...estimate.time) : undefined].filter(Boolean).join(', ')}`
}

const withCode = (config: Pick<MonitorConfig, 'code_repository'> | null) =>
  config?.code_repository ? ' and code access' : ''

/** The line under a failure: what a Reanalyze would cost, or why nobody can say. */
export function estimateLine(estimate: Estimate): string {
  const text = estimateText(estimate)
  return text
    ? `Reanalyze runs the whole pipeline again: ${text}, based on ${plural(estimate.count, 'analysis', 'analyses')} with this model.`
    : `No estimate yet: fewer than ${MIN_ESTIMATE} completed analyses with this model.`
}

/** What starts an analysis: each asks before it spends anything. */
export type SpendKind = 'reanalyze' | 'analyze' | 'start' | 'resume'

/** What a new analysis costs, and the question that comes before it starts (`false`: the person said no). */
export interface SpendGuide {
  /** The estimate line a failure ends with. */
  estimate: string
  confirm: (kind: SpendKind) => Promise<boolean>
}

export interface SpendDialog {
  title: string
  description: string
  details: string[]
  confirmLabel: string
  /** The provider check can say when it is fixed, so the dialog offers Check again. */
  recheck: boolean
}

const ANYWAY: Record<SpendKind, string> = {
  reanalyze: 'Reanalyze anyway',
  analyze: 'Analyze anyway',
  start: 'Save and start anyway',
  resume: 'Resume anyway',
}

/**
 * The confirmation before an analysis can start. Triage that cannot run is said first; otherwise Reanalyze and
 * Analyze say what the analysis would cost. `null`: nothing to ask.
 */
export function spendDialog(input: {
  kind: SpendKind
  config: MonitorConfig | null
  limits: MonitorLimits | undefined
  estimate: Estimate
  problem: TriageProblem | undefined
  now: number
}): SpendDialog | null {
  const { kind, config, limits, estimate, problem, now } = input
  const text = estimateText(estimate)
  const access = withCode(config)

  if (problem) {
    const credits = problem.code === OUT_OF_CREDITS
    return {
      title: 'Triage is unavailable',
      description: credits
        ? `Jev answered HTTP 402 (no credits) on its last triage, ${ago(problem.at, now)}. A new analysis would fail at triage again.`
        : `Jev can't run triage: ${reasonWords(problem.code)}, checked ${ago(problem.at, now)}. A new analysis would fail at triage again.`,
      details: [
        'Nothing is spent on the investigation when triage fails',
        credits ? 'Add credits in TypeSafe billing, then analyze again' : triageHint(problem.code),
        ...(text ? [`Estimate if it worked: ${text}`] : []),
      ],
      confirmLabel: ANYWAY[kind],
      recheck: !credits,
    }
  }
  if (kind === 'start' || kind === 'resume' || (kind === 'analyze' && !text)) return null

  const again = kind === 'reanalyze' ? ' again' : ''
  const where = config?.code_repository ? `reading code in ${config.code_repository}` : 'without code access'
  const model = config ? ` with ${modelWithLevel(config.model)}, ${where}` : ''
  const caps = limits ? investigationCaps(limits, Boolean(config?.code_repository)) : undefined
  return {
    title: kind === 'reanalyze' ? 'Reanalyze this session?' : 'Analyze this session?',
    description: `The whole pipeline runs${again}: collection, triage, and investigation if Jev answers needs_investigation${model}.`,
    details: text
      ? [
          `Estimate ${text}`,
          `Based on ${plural(estimate.count, 'completed analysis', 'completed analyses')} with this model${access}`,
          ...(kind === 'reanalyze'
            ? ['The earlier analysis stays in the list; the new one is linked to it as its replacement']
            : []),
        ]
      : [
          `No estimate yet: fewer than ${MIN_ESTIMATE} completed analyses with this model${access}`,
          ...(caps
            ? [`Each analysis is capped at ${plural(caps.steps, 'step')} and ${tokenCap(caps.totalTokens)} tokens`]
            : []),
          'Cost is shown on the analysis once it ends, never guessed',
        ],
    confirmLabel: kind === 'reanalyze' ? 'Reanalyze' : 'Analyze',
    recheck: false,
  }
}

// ── The history, grouped by the turn it observed ───────────────────────

/** An analysis alone, or two or more of one observed turn (a reanalysis is read next to what it replaces). */
export type HistoryItem =
  | { kind: 'row'; record: AnalysisRecord }
  | {
      kind: 'group'
      /** The `observation_key` the members share. */
      key: string
      /** Every analysis of the turn, newest first: the tally counts them all, whatever the filter hides. */
      members: AnalysisRecord[]
      /** The members the filter lets through, newest first; never empty. */
      shown: AnalysisRecord[]
    }

/**
 * Groups the list by `observation_key` and orders items by their newest member. Single analyses stay plain rows.
 * The filter narrows the rows; a group none of whose rows pass disappears.
 */
export function groupHistory(
  records: readonly AnalysisRecord[],
  filter: Filter,
  reviews: ReviewIndex | null = null,
): HistoryItem[] {
  const newestFirst = [...records].sort((a, b) => b.created_at - a.created_at)
  const byKey = new Map<string, AnalysisRecord[]>()
  for (const record of newestFirst)
    byKey.set(record.observation_key, [...(byKey.get(record.observation_key) ?? []), record])
  const items: HistoryItem[] = []
  const placed = new Set<string>()
  for (const record of newestFirst) {
    if (placed.has(record.observation_key)) continue
    placed.add(record.observation_key)
    const members = byKey.get(record.observation_key) ?? [record]
    const shown = members.filter((member) =>
      reviews && filter === 'suggestions' ? needsReview(member, reviews) : matchesFilter(member, filter),
    )
    if (shown.length === 0) continue
    items.push(
      members.length === 1
        ? { kind: 'row', record: members[0] }
        : { kind: 'group', key: record.observation_key, members, shown },
    )
  }
  return items
}

/** `9 analyses: 4 with suggestions, 5 failed`, over every member of the turn. */
export function groupTally(members: readonly AnalysisRecord[]): string {
  const count = (test: (record: AnalysisRecord) => boolean) => members.filter(test).length
  const parts = [
    [count((r) => r.status === 'completed' && r.counters.suggestions > 0), 'with suggestions'],
    [count((r) => r.status === 'completed' && r.counters.suggestions === 0), 'without suggestions'],
    [count((r) => r.status === 'failed'), 'failed'],
    [count((r) => r.status === 'cancelled'), 'cancelled'],
    [count((r) => isActive(r.status)), 'running'],
  ]
    .filter(([n]) => n !== 0)
    .map(([n, label]) => `${n} ${label}`)
  return `${plural(members.length, 'analysis', 'analyses')}: ${parts.join(', ')}`
}

/** `turn t_e864… · e2e_36328da3`: which turn of which session a group observed. */
export function groupSource(record: AnalysisRecord): string {
  return `turn ${shortId(record.turn_id, 6)} · ${record.session_id}`
}

/** What went wrong in words, for a failed row; `undefined` for any other status. */
export function rowCause(record: AnalysisRecord): string | undefined {
  if (record.status !== 'failed' || !record.failure) return undefined
  const headline = failureHeadline(record.failure)
  return failureCode(record.failure) === 'judge_out_of_credits' ? `${headline} (402)` : headline
}

/**
 * `$1.27 · 6m 20s · reads code`: what an analysis of a group cost, how long it took and whether it read code.
 * Unknown stays unknown: an investigation that reported no cost says so, one that never ran says nothing about it.
 */
export function memberMeta(record: AnalysisRecord): string {
  const ran = record.analyst !== undefined
  const cost =
    record.usage.llm_cost_usd !== undefined
      ? formatCostShort(record.usage.llm_cost_usd)
      : ran || record.status === 'failed'
        ? 'no cost reported'
        : undefined
  const took = record.completed_at === undefined ? undefined : span(record.completed_at - record.created_at)
  return [cost, took, record.code_root ? 'reads code' : 'no code'].filter(Boolean).join(' · ')
}

/** `Today 13:21`: when a row was created, with its day. */
export function rowTime(record: AnalysisRecord, now: number): string {
  return dayClock(record.created_at, now)
}

// ── What people decided about the suggestions, in the list ─────────────

/** `eval::reviews` by analysis: the counts of every analysis with suggestions, and the rows somebody acted on. */
export interface ReviewIndex {
  summaries: ReadonlyMap<string, ReviewSummary>
  rows: ReadonlyMap<string, readonly SuggestionReview[]>
}

export function indexReviews(response: ReviewsResponse | null): ReviewIndex | null {
  if (!response) return null
  const rows = new Map<string, SuggestionReview[]>()
  for (const row of response.reviews) rows.set(row.evaluation_id, [...(rows.get(row.evaluation_id) ?? []), row])
  return { summaries: new Map(response.summaries.map((summary) => [summary.evaluation_id, summary])), rows }
}

/** At least one suggestion nobody has acted on. */
export function needsReview(record: Pick<AnalysisRecord, 'evaluation_id'>, reviews: ReviewIndex): boolean {
  return (reviews.summaries.get(record.evaluation_id)?.new ?? 0) > 0
}

/** The "To review" filter: `To review 2`; without the review counts it is the old "Suggestions". */
export function filterLabel(
  filter: Filter,
  label: string,
  records: readonly AnalysisRecord[] | null,
  reviews: ReviewIndex | null,
): string {
  if (filter !== 'suggestions' || !reviews) return label
  const count = records?.filter((record) => needsReview(record, reviews)).length ?? 0
  return count > 0 ? `To review ${count}` : 'To review'
}

const STATUS_WORD = {
  new: 'new',
  accepted: 'accepted',
  in_progress: 'in progress',
  shipped: 'shipped',
  rejected: 'rejected',
  duplicate: 'duplicate',
} as const

const STATUSES = Object.keys(STATUS_WORD) as Array<keyof typeof STATUS_WORD>

/** `1 suggestion · new`, `2 suggestions · 1 new`, `1 suggestion · shipped`; `undefined` without suggestions. */
export function reviewLine(
  record: Pick<AnalysisRecord, 'evaluation_id'>,
  reviews: ReviewIndex | null,
): string | undefined {
  const summary = reviews?.summaries.get(record.evaluation_id)
  if (!summary || summary.suggestions === 0) return undefined
  const noun = plural(summary.suggestions, 'suggestion')
  const present = STATUSES.filter((status) => summary[status] > 0)
  if (present.length === 1) return `${noun} · ${STATUS_WORD[present[0]]}`
  return summary.new > 0 ? `${noun} · ${summary.new} new` : `${noun} · all reviewed`
}

/** `S1 in progress · S2 new`: where each suggestion stands, once somebody has acted on one. */
export function reviewMeta(
  record: Pick<AnalysisRecord, 'evaluation_id'>,
  reviews: ReviewIndex | null,
): string | undefined {
  const summary = reviews?.summaries.get(record.evaluation_id)
  if (!summary || summary.new === summary.suggestions) return undefined
  const rows = reviews?.rows.get(record.evaluation_id) ?? []
  return Array.from({ length: summary.suggestions }, (_, index) => {
    const status = rows.find((row) => row.suggestion_index === index)?.lifecycle.status ?? 'new'
    return `S${index + 1} ${STATUS_WORD[status]}`
  }).join(' · ')
}
