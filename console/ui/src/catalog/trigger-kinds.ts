/**
 * What a binding IS, read from its trigger type and config.
 *
 * A trigger row is only useful if it says the thing the operator recognizes:
 * an http binding is `GET /users/:id`, a cron binding is its schedule, a
 * queue subscriber is its topic. This module owns that reading, one entry per
 * family, so the page and the detail pane agree and an unknown type still
 * gets a sane line instead of a blank.
 *
 * Families are matched on the type id the worker publishes today. Unknown
 * ids fall through to the generic reading rather than being hidden: the type
 * set is open, and a worker that ships a new type must still list.
 */

import { describeCron } from './cron'
import type { RegisteredTrigger } from './engine'

export type Family =
  | 'http'
  | 'cron'
  | 'queue'
  | 'state'
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

const FAMILIES: { match: (typeId: string) => boolean; spec: FamilySpec }[] = [
  {
    match: (t) => t === 'http',
    spec: { family: 'http', label: 'http', tone: 'accent' },
  },
  {
    match: (t) => t === 'cron' || t === 'timer',
    spec: { family: 'cron', label: 'schedule', tone: 'warn' },
  },
  {
    match: (t) => t === 'durable:subscriber' || t.startsWith('queue'),
    spec: { family: 'queue', label: 'queue', tone: 'ok' },
  },
  {
    match: (t) => t === 'state',
    spec: { family: 'state', label: 'state', tone: 'ok' },
  },
  {
    match: (t) => t === 'stream' || t.startsWith('stream:'),
    spec: { family: 'stream', label: 'stream', tone: 'accent' },
  },
  {
    match: (t) => t.startsWith('harness::hook::'),
    spec: { family: 'hook', label: 'hook', tone: 'ink' },
  },
  {
    match: (t) => t.startsWith('console:'),
    spec: { family: 'asset', label: 'console asset', tone: 'ink' },
  },
]

export function familyOf(typeId: string): FamilySpec {
  for (const entry of FAMILIES) {
    if (entry.match(typeId)) return entry.spec
  }
  return { family: 'other', label: 'event', tone: 'ink' }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function text(config: unknown, key: string): string | undefined {
  if (!isRecord(config)) return undefined
  const value = config[key]
  return typeof value === 'string' && value ? value : undefined
}

export interface HttpBinding {
  method: string
  path: string
  /** `:param` names in declaration order. */
  params: string[]
}

/** The http family's config, or null when the binding is not http. */
export function httpBinding(trigger: RegisteredTrigger): HttpBinding | null {
  if (familyOf(trigger.trigger_type).family !== 'http') return null
  const path = text(trigger.config, 'api_path') ?? ''
  const params = [...path.matchAll(/:([a-zA-Z_][a-zA-Z0-9_]*)/g)].map(
    (m) => m[1],
  )
  return {
    method: (text(trigger.config, 'http_method') ?? 'GET').toUpperCase(),
    path: path.startsWith('/') ? path : `/${path}`,
    params,
  }
}

/** The queue topic a subscriber consumes, under either config key. */
export function queueTopic(trigger: RegisteredTrigger): string | undefined {
  if (familyOf(trigger.trigger_type).family !== 'queue') return undefined
  return text(trigger.config, 'queue') ?? text(trigger.config, 'topic')
}

export function cronExpression(trigger: RegisteredTrigger): string | undefined {
  if (familyOf(trigger.trigger_type).family !== 'cron') return undefined
  return text(trigger.config, 'expression')
}

/**
 * The one line that names a binding in the list: what it listens to, in the
 * words of its family. Falls back to the compact config, then the type id,
 * so a row is never blank.
 */
export function summarize(trigger: RegisteredTrigger): string {
  const http = httpBinding(trigger)
  if (http) return `${http.method} ${http.path}`

  const expression = cronExpression(trigger)
  if (expression) return describeCron(expression) ?? expression

  const topic = queueTopic(trigger)
  if (topic) return topic

  const family = familyOf(trigger.trigger_type).family
  if (family === 'state') {
    const scope = text(trigger.config, 'scope')
    const key = text(trigger.config, 'key')
    if (scope && key) return `${scope}/${key}`
    if (scope) return `${scope}/*`
    if (key) return `*/${key}`
    return 'any state write'
  }
  if (family === 'stream') {
    const stream = text(trigger.config, 'stream_name')
    const group = text(trigger.config, 'group_id')
    if (stream) return group ? `${stream} · ${group}` : stream
  }
  if (family === 'asset') {
    const path = text(trigger.config, 'path')
    if (path) return path
  }
  if (family === 'hook') {
    return trigger.trigger_type.replace('harness::hook::', 'hook: ')
  }

  const summary = trigger.config_summary
  if (summary && summary !== '{}') return summary
  return trigger.trigger_type
}

/**
 * Config fields worth a chip in the detail pane, in a stable order. Unknown
 * fields are not listed here on purpose: they stay visible in the raw config
 * block, which every binding shows.
 */
export function configChips(
  trigger: RegisteredTrigger,
): { label: string; value: string }[] {
  if (!isRecord(trigger.config)) return []
  const config = trigger.config
  const chips: { label: string; value: string }[] = []
  const push = (key: string, label: string) => {
    const value = config[key]
    if (typeof value === 'string' && value) chips.push({ label, value })
    else if (typeof value === 'number')
      chips.push({ label, value: String(value) })
  }
  push('scope', 'scope')
  push('key', 'key')
  push('queue', 'queue')
  push('topic', 'topic')
  push('stream_name', 'stream')
  push('group_id', 'group')
  push('configuration_id', 'configuration')
  push('max_retries', 'retries')
  push('backoff_ms', 'backoff ms')
  push('on_error', 'on error')
  push('condition_function_id', 'if')
  return chips
}
