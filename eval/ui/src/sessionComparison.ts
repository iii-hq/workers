import type { SessionMeta } from './types'

export type ComparisonMetricKey =
  | 'total_tokens'
  | 'input_tokens'
  | 'output_tokens'
  | 'cache_read_tokens'
  | 'cache_write_tokens'
  | 'reasoning_tokens'
  | 'subject_cost_usd'
  | 'trace_duration_ms'
  | 'tokens_per_generation'
  | 'cost_per_generation_usd'
  | 'function_calls'
  | 'function_call_errors'
  | 'function_error_rate'
  | 'error_span_count'
  | 'sessions'
  | 'descendants'
  | 'max_depth'
  | 'generations'
  | 'trace_count'
  | 'span_count'
  | 'compacted_sessions'
  | 'context_total_tokens'
  | 'context_usable_tokens'
  | 'context_free_tokens'
  | 'context_occupancy'

export interface ComparisonMetric {
  key: ComparisonMetricKey
  label: string
  format?: 'number' | 'cost' | 'duration' | 'ratio'
}

export const metricGroups: Array<{ label: string; metrics: ComparisonMetric[] }> = [
  {
    label: 'Efficiency',
    metrics: [
      { key: 'total_tokens', label: 'total tokens' },
      { key: 'input_tokens', label: 'input tokens' },
      { key: 'output_tokens', label: 'output tokens' },
      { key: 'cache_read_tokens', label: 'cache read tokens' },
      { key: 'cache_write_tokens', label: 'cache write tokens' },
      { key: 'reasoning_tokens', label: 'reasoning tokens' },
      { key: 'subject_cost_usd', label: 'subject cost', format: 'cost' },
      { key: 'trace_duration_ms', label: 'observed trace duration', format: 'duration' },
      { key: 'tokens_per_generation', label: 'tokens / generation', format: 'number' },
      { key: 'cost_per_generation_usd', label: 'subject cost / generation', format: 'cost' },
    ],
  },
  {
    label: 'Reliability',
    metrics: [
      { key: 'function_call_errors', label: 'function-call errors' },
      { key: 'function_error_rate', label: 'function errors / call', format: 'ratio' },
      { key: 'error_span_count', label: 'error spans' },
    ],
  },
  {
    label: 'Orchestration',
    metrics: [
      { key: 'sessions', label: 'sessions' },
      { key: 'descendants', label: 'descendants' },
      { key: 'max_depth', label: 'maximum depth' },
      { key: 'generations', label: 'generations' },
      { key: 'function_calls', label: 'function calls' },
      { key: 'trace_count', label: 'traces' },
      { key: 'span_count', label: 'spans' },
    ],
  },
  {
    label: 'Context',
    metrics: [
      { key: 'context_total_tokens', label: 'latest snapshot total' },
      { key: 'context_usable_tokens', label: 'latest snapshot usable' },
      { key: 'context_free_tokens', label: 'latest snapshot free' },
      { key: 'context_occupancy', label: 'latest snapshot occupancy', format: 'ratio' },
      { key: 'compacted_sessions', label: 'sessions with compaction' },
    ],
  },
]

export function rootSessions(sessions: SessionMeta[]): SessionMeta[] {
  return sessions.filter((session) => {
    const metadata = session.metadata
    return !metadata || !Object.prototype.hasOwnProperty.call(metadata, 'parent_session_id')
  })
}

export function toggleSession(
  selected: string[],
  sessionId: string,
  max = 5,
): string[] {
  if (selected.includes(sessionId)) return selected.filter((id) => id !== sessionId)
  if (selected.length >= max) return selected
  return [...selected, sessionId]
}

export function formatMetricValue(
  value: number | null | undefined,
  format: ComparisonMetric['format'] = 'number',
): string {
  if (value === null || value === undefined || !Number.isFinite(value)) return '—'
  if (format === 'cost') return `$${value.toFixed(6)}`
  if (format === 'duration') {
    return value >= 1000 ? `${(value / 1000).toFixed(2)} s` : `${Math.round(value)} ms`
  }
  if (format === 'ratio') return `${(value * 100).toFixed(1)}%`
  return Number.isInteger(value) ? value.toLocaleString() : value.toFixed(2)
}

export function formatDelta(
  delta: { absolute: number | null; percent: number | null } | undefined,
  format: ComparisonMetric['format'] = 'number',
): string {
  if (!delta || delta.absolute === null || delta.absolute === undefined) return '—'
  const absolute = formatMetricValue(delta.absolute, format)
  const signedAbsolute = delta.absolute > 0 ? `+${absolute}` : absolute
  const percent =
    delta.percent === null || delta.percent === undefined
      ? '—'
      : `${delta.percent > 0 ? '+' : ''}${delta.percent.toFixed(1)}%`
  return `Δ ${signedAbsolute} · ${percent}`
}

export function formatDate(value: number): string {
  return new Date(value).toLocaleString()
}
