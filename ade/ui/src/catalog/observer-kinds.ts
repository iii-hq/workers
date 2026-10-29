/**
 * What an observer IS, read from the function it observes, its args and its
 * filters.
 *
 * A row is only useful if it says the thing the operator recognizes: an
 * observer of `http::on-request` is `GET /users/:id`, one of `cron::on-tick`
 * is its schedule, one of `queue::on-message` is its topic, one of
 * `state::on-changed` is the scope and key its filters select. This module
 * owns that reading, one entry per family, so the page and the detail pane
 * agree and an unknown function still gets a sane line instead of a blank.
 *
 * Families are matched on the observed function id. Unknown ids fall
 * through to the generic reading rather than being hidden: every function
 * is observable, and a worker that ships a new source must still list.
 */

import { describeCron } from './cron'
import type { ObserverInfo } from './engine'

export type Family =
  | 'http'
  | 'cron'
  | 'queue'
  | 'state'
  | 'database'
  | 'stream'
  | 'hook'
  | 'asset'
  | 'other'

/** Tone drives the row chip color; it maps to the console's own tokens. */
export type Tone = 'accent' | 'warn' | 'ok' | 'alert' | 'ink'

export interface FamilySpec {
  family: Family
  label: string
  tone: Tone
}

/** The functions whose observers the page reads specially. */
export const HTTP_FN = 'http::on-request'
export const CRON_FN = 'cron::on-tick'
export const TIMER_FN = 'harness::on-timer'
export const QUEUE_FN = 'queue::on-message'
export const STATE_FN = 'state::on-changed'
export const DATABASE_FN = 'database::on-row-changed'
export const CONFIGURATION_FN = 'configuration::on-updated'

const FAMILIES: { match: (fnId: string) => boolean; spec: FamilySpec }[] = [
  {
    match: (f) => f === HTTP_FN,
    spec: { family: 'http', label: 'http', tone: 'accent' },
  },
  {
    match: (f) => f === CRON_FN || f === TIMER_FN,
    spec: { family: 'cron', label: 'schedule', tone: 'warn' },
  },
  {
    match: (f) => f === QUEUE_FN,
    spec: { family: 'queue', label: 'queue', tone: 'ok' },
  },
  {
    match: (f) => f === STATE_FN,
    spec: { family: 'state', label: 'state', tone: 'ok' },
  },
  {
    match: (f) => f === DATABASE_FN,
    spec: { family: 'database', label: 'database', tone: 'ok' },
  },
  {
    match: (f) => f.startsWith('stream::on-'),
    spec: { family: 'stream', label: 'stream', tone: 'accent' },
  },
  {
    match: (f) => f.startsWith('harness::hook::'),
    spec: { family: 'hook', label: 'hook', tone: 'ink' },
  },
  {
    match: (f) => f.startsWith('console::ui::'),
    spec: { family: 'asset', label: 'ADE asset', tone: 'ink' },
  },
]

export function familyOf(functionId: string): FamilySpec {
  for (const entry of FAMILIES) {
    if (entry.match(functionId)) return entry.spec
  }
  return { family: 'other', label: 'event', tone: 'ink' }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function text(source: unknown, key: string): string | undefined {
  if (!isRecord(source)) return undefined
  const value = source[key]
  return typeof value === 'string' && value ? value : undefined
}

/**
 * The value a filter pins a field to: `{ 'input.scope': { $eq: 'orders' } }`
 * reads as `orders` for `input.scope`. Anything but a plain `$eq` (or a
 * one-element `$in`) is not a single value and reads as absent.
 */
export function filterValue(
  observer: Pick<ObserverInfo, 'filters'>,
  field: string,
): string | undefined {
  const clause = observer.filters[field]
  if (typeof clause === 'string') return clause
  if (!isRecord(clause)) return undefined
  if (typeof clause.$eq === 'string') return clause.$eq
  if (
    Array.isArray(clause.$in) &&
    clause.$in.length === 1 &&
    typeof clause.$in[0] === 'string'
  ) {
    return clause.$in[0]
  }
  return undefined
}

export interface HttpBinding {
  method: string
  path: string
  /** `:param` names in declaration order. */
  params: string[]
}

/** The route an `http::on-request` observer holds, or null for any other. */
export function httpBinding(observer: ObserverInfo): HttpBinding | null {
  if (observer.function_id !== HTTP_FN) return null
  const path = text(observer.args, 'path') ?? ''
  const params = [...path.matchAll(/:([a-zA-Z_][a-zA-Z0-9_]*)/g)].map(
    (m) => m[1],
  )
  return {
    method: (text(observer.args, 'method') ?? 'GET').toUpperCase(),
    path: path.startsWith('/') ? path : `/${path}`,
    params,
  }
}

/** The topic a `queue::on-message` observer consumes. */
export function queueTopic(observer: ObserverInfo): string | undefined {
  if (observer.function_id !== QUEUE_FN) return undefined
  return text(observer.args, 'topic')
}

/** The expression a `cron::on-tick` observer holds. */
export function cronExpression(observer: ObserverInfo): string | undefined {
  if (observer.function_id !== CRON_FN) return undefined
  return text(observer.args, 'expression')
}

/** When a `harness::on-timer` observer fires (`args.at`, epoch ms). */
function timerAt(observer: ObserverInfo): number | undefined {
  if (observer.function_id !== TIMER_FN) return undefined
  const at = observer.args.at
  return typeof at === 'number' && Number.isFinite(at) ? at : undefined
}

/**
 * The one line that names an observer in the list: what it listens to, in
 * the words of its family. Falls back to the session it follows, then the
 * observed function id, so a row is never blank.
 */
export function summarize(observer: ObserverInfo): string {
  const http = httpBinding(observer)
  if (http) return `${http.method} ${http.path}`

  const expression = cronExpression(observer)
  if (expression) return describeCron(expression) ?? expression

  const at = timerAt(observer)
  if (at !== undefined) return `at ${new Date(at).toLocaleString()}`

  const topic = queueTopic(observer)
  if (topic) return topic

  const family = familyOf(observer.function_id).family
  if (family === 'state') {
    const scope = filterValue(observer, 'input.scope')
    const key = filterValue(observer, 'input.key')
    if (scope && key) return `${scope}/${key}`
    if (scope) return `${scope}/*`
    if (key) return `*/${key}`
    return 'any state write'
  }
  if (family === 'database') {
    const db = text(observer.args, 'db') ?? '*'
    const table = text(observer.args, 'table') ?? '*'
    const ops = Array.isArray(observer.args.ops)
      ? observer.args.ops.filter((op): op is string => typeof op === 'string')
      : []
    return `${db}.${table}${ops.length ? ` · ${ops.join(', ')}` : ''}`
  }
  if (family === 'stream') {
    const stream = filterValue(observer, 'input.stream_name')
    const group = filterValue(observer, 'input.group_id')
    if (stream) return group ? `${stream} · ${group}` : stream
  }
  if (family === 'asset') {
    const path = text(observer.args, 'path')
    if (path) return path
  }
  if (family === 'hook') {
    return observer.function_id.replace('harness::hook::', 'hook: ')
  }

  // Session-scoped delivery (a console tab or sub-agent listening): say that,
  // not the raw args JSON.
  const sessionId =
    text(observer.args, 'session_id') ?? filterValue(observer, 'input.session_id')
  if (sessionId) {
    return `session ${sessionId.length > 18 ? `…${sessionId.slice(-12)}` : sessionId}`
  }
  if (observer.function_id === CONFIGURATION_FN) {
    const id = filterValue(observer, 'input.id')
    if (id) return `entry ${id}`
  }
  return observer.function_id
}

/**
 * Console and engine plumbing: per-tab delivery handlers (`iii::` prefix),
 * injected-UI assets, configuration hot-reload handlers, UI content
 * functions. All real, none of them what an operator opens this page to see
 * — they fold into one collapsed group at the bottom instead of burying the
 * rest.
 */
export function isPlumbing(observer: ObserverInfo): boolean {
  const target = observer.target.function_id
  if (target.startsWith('iii::')) return true
  if (observer.function_id.startsWith('console::ui::')) return true
  if (target.endsWith('::ui-content')) return true
  if (
    observer.function_id === CONFIGURATION_FN &&
    /on[-_]config[-_]change/.test(target)
  ) {
    return true
  }
  return false
}

/**
 * Args and filters worth a chip in the detail pane, in a stable order.
 * Unknown fields are not listed here on purpose: they stay visible in the
 * raw args and filters blocks every observer shows.
 */
export function observerChips(
  observer: ObserverInfo,
): { label: string; value: string }[] {
  const chips: { label: string; value: string }[] = []
  const arg = (key: string, label: string) => {
    const value = observer.args[key]
    if (typeof value === 'string' && value) chips.push({ label, value })
    else if (typeof value === 'number')
      chips.push({ label, value: String(value) })
  }
  const filter = (field: string, label: string) => {
    const value = filterValue(observer, field)
    if (value) chips.push({ label, value })
  }
  filter('input.scope', 'scope')
  filter('input.key', 'key')
  arg('topic', 'topic')
  arg('group', 'group')
  arg('db', 'db')
  arg('table', 'table')
  filter('input.stream_name', 'stream')
  filter('input.group_id', 'group')
  filter('input.id', 'configuration')
  arg('max_retries', 'retries')
  arg('backoff_ms', 'backoff ms')
  arg('priority', 'priority')
  arg('on_error', 'on error')
  arg('timeout_ms', 'timeout ms')
  return chips
}
