/**
 * The one-line facts a settled tool row shows at its trailing edge, and the
 * one-line reason a failed row shows under its title. Both read the raw
 * output a card is allowed to display (post-`redactRaw`), so nothing here
 * can leak a value the card itself would mask.
 */

import { formatDuration } from '@iii-dev/console-ui/format'

/**
 * `87ms`, `1.2s`, `2m 05s` — the console's shared duration scale, plus the
 * traces page's `μs` below a millisecond: a lot of what an agent dispatches
 * is engine-local and finishes in microseconds, where `0ms` reads like the
 * call never happened.
 */
export function formatRowDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '0μs'
  if (ms < 1) return `${Math.max(1, Math.round(ms * 1000))}μs`
  return formatDuration(ms)
}

function unwrapEnvelope(output: unknown): unknown {
  if (!output || typeof output !== 'object' || Array.isArray(output))
    return output
  const o = output as Record<string, unknown>
  if (Array.isArray(o.content) && 'details' in o) return o.details
  return output
}

/** `files` → `file`, `matches` → `match`, `entries` → `entry`. */
function singular(noun: string): string {
  if (/ies$/.test(noun)) return noun.replace(/ies$/, 'y')
  if (/(ch|sh|ss|x)es$/.test(noun)) return noun.replace(/es$/, '')
  return noun.replace(/s$/, '')
}

function plural(noun: string): string {
  if (/[^aeiou]y$/.test(noun)) return noun.replace(/y$/, 'ies')
  if (/(ch|sh|ss|x)$/.test(noun)) return `${noun}es`
  return `${noun}s`
}

function pluralize(n: number, noun: string): string {
  return `${n.toLocaleString('en-US')} ${n === 1 ? noun : plural(noun)}`
}

/**
 * A short count the reader can scan without opening the row: `4 results`
 * for a list, `3 files` for a single array field, `exit 0` for a process.
 * Null when the output has no obvious countable shape — a wrong guess is
 * worse than no summary.
 */
export function resultSummary(output: unknown): string | null {
  const value = unwrapEnvelope(output)
  if (value === null || value === undefined) return null
  if (Array.isArray(value)) return pluralize(value.length, 'result')
  if (typeof value !== 'object') return null
  const record = value as Record<string, unknown>
  if ('error' in record) return null

  const exit = record.exit_code ?? record.exitCode
  if (typeof exit === 'number') return `exit ${exit}`

  const arrays = Object.entries(record).filter(([, v]) => Array.isArray(v))
  if (arrays.length === 1) {
    const [key, list] = arrays[0] as [string, unknown[]]
    return pluralize(list.length, singular(key.replace(/_/g, ' ')))
  }

  for (const key of ['count', 'total', 'matches']) {
    const n = record[key]
    if (typeof n === 'number') return pluralize(n, 'result')
  }
  return null
}

const SUBTITLE_MAX = 160

function clip(text: string): string {
  const line = text.replace(/\s+/g, ' ').trim()
  return line.length > SUBTITLE_MAX
    ? `${line.slice(0, SUBTITLE_MAX - 1)}…`
    : line
}

/**
 * Why a call failed, from the `{ error }` envelope soft failures ride on. A
 * gate denial names who denied it; anything else surfaces the message, then
 * the kind, then the code. Null when the output carries no error.
 */
export function errorSubtitle(output: unknown): string | null {
  if (!output || typeof output !== 'object' || Array.isArray(output))
    return null
  const error = (output as { error?: unknown }).error
  if (error === undefined) return null
  if (typeof error === 'string') return clip(error) || 'Failed'
  if (!error || typeof error !== 'object') return 'Failed'
  const e = error as Record<string, unknown>
  const details = e.details
  if (
    details &&
    typeof details === 'object' &&
    !Array.isArray(details) &&
    (details as Record<string, unknown>).status === 'denied'
  ) {
    const by = (details as Record<string, unknown>).denied_by
    const reason = (details as Record<string, unknown>).reason
    const who = typeof by === 'string' && by ? ` by ${by}` : ''
    const why = typeof reason === 'string' && reason ? ` · ${clip(reason)}` : ''
    return `Denied${who}${why}`
  }
  for (const key of ['message', 'kind', 'code']) {
    const v = e[key]
    if (typeof v === 'string' && v.trim()) return clip(v)
  }
  return 'Failed'
}
