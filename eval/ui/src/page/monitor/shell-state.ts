// Pure logic of the Monitor shell: the status card, the queue and cost lines,
// the history rows, the filter copy, notices and how a palette or command
// context reaches the page. No React, no host — unit-tested.
import { formatBytes } from '@iii-dev/console-ui/format'
import { type Filter, isActive, monitorSummary, rowDetail, statusPresentation, type Tone } from '../../model'
import type { AnalysisRecord, CapacityRejection, MonitorState } from '../../types'
import { formatCostShort, parseSizes } from './detail/present'

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

export function emptyFilterTitle(filter: Filter): string {
  switch (filter) {
    case 'active':
      return 'No active analyses'
    case 'suggestions':
      return 'No analyses with suggestions'
    case 'failed':
      return 'No failed analyses'
    case 'all':
      return 'No analyses yet'
  }
}

export type CardState = 'loading' | 'unconfigured' | 'unavailable' | 'paused' | 'observing'

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
  return state.observer_bound ? 'observing' : 'unavailable'
}

export const CARD_LABEL: Record<Exclude<CardState, 'loading'>, string> = {
  unconfigured: 'Not configured',
  unavailable: 'Unavailable',
  paused: 'Paused',
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
  if (!state || !rejection) return undefined
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

/** `19:41`, local time. */
export function clock(ms: number): string {
  const date = new Date(ms)
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
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
