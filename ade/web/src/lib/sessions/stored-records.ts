/**
 * Readers for transcript records and names written before 0.26, so stored
 * conversations keep rendering:
 *
 * - the harness's dispatch tool, `agent_call`, recorded as `agent_trigger`;
 * - the observer activity record, `observer_fired`, recorded as
 *   `trigger_fired` with `subscription_id`, `trigger_id`, `trigger_type` and
 *   `config` in place of `observer_id`, `function_id`, `args` and `filters`;
 * - activity entry ids, `e_obs<kind>_…`, recorded as `e_trig<kind>_…`.
 *
 * Everything here reads; nothing writes the old names.
 */

import type { ObserverFiredData } from '@/types/chat'

/** The harness's model-facing dispatch tool. */
export const AGENT_CALL = 'agent_call'
const STORED_AGENT_CALL = 'agent_trigger'

/** Whether a recorded function call is the harness dispatch wrapper. */
export function isAgentCall(functionId: string | undefined): boolean {
  return functionId === AGENT_CALL || functionId === STORED_AGENT_CALL
}

/** The custom type of the harness's observer activity record. */
export const OBSERVER_FIRED_CUSTOM_TYPE = 'observer_fired'
const STORED_FIRED_CUSTOM_TYPE = 'trigger_fired'

/** Whether a custom record is an observer activity record. */
export function isObserverFiredType(customType: string): boolean {
  return (
    customType === OBSERVER_FIRED_CUSTOM_TYPE ||
    customType === STORED_FIRED_CUSTOM_TYPE
  )
}

/**
 * The prefix of an observer activity entry id of `kind` (`fired`, `skip`,
 * `expired`, `stale`, `unregistered`): `e_obs<kind>_`, or `e_trig<kind>_` on
 * records written before 0.26. Returns the matched prefix, or `null`.
 */
export function activityIdPrefix(
  entryId: string,
  kind: 'fired' | 'skip' | 'expired' | 'stale' | 'unregistered',
): string | null {
  for (const prefix of [`e_obs${kind}_`, `e_trig${kind}_`]) {
    if (entryId.startsWith(prefix)) return prefix
  }
  return null
}

const recordOf = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null

const stringOf = (value: unknown): string | undefined =>
  typeof value === 'string' && value.length > 0 ? value : undefined

/** The functions that replaced the engine's and the workers' event types. */
const RENAMED_SOURCES: Readonly<Record<string, string>> = {
  cron: 'cron::on-tick',
  http: 'http::on-request',
  'durable:subscriber': 'queue::on-message',
  'database::row-changed': 'database::on-row-changed',
  state: 'state::on-changed',
  subscribe: 'publish',
  log: 'observability::on-log',
  trace: 'observability::on-span',
  configuration: 'configuration::on-updated',
  'engine::functions-available': 'engine::on-functions-changed',
  'engine::workers-available': 'engine::on-workers-changed',
  stream: 'stream::on-changed',
  'stream:join': 'stream::on-join',
  'stream:leave': 'stream::on-leave',
  'compose-operation': 'compose::on-operation-progress',
  timer: 'harness::on-timer',
  'harness::triggers-changed': 'harness::on-observers-changed',
}

/** Functions whose observers read a stored config as their args. */
const ARG_SOURCES = new Set([
  'cron::on-tick',
  'http::on-request',
  'queue::on-message',
  'database::on-row-changed',
  'harness::on-timer',
])

/**
 * The function a stored event type names: the fixed table, then the naming
 * rule (`W::X` → `W::on-X`, `a:b` → `a::on-b`; a last segment already
 * starting with `on-` stays).
 */
export function functionOfStoredType(type: string): string {
  const renamed = RENAMED_SOURCES[type]
  if (renamed) return renamed
  if (type.includes('::')) {
    const at = type.lastIndexOf('::')
    const last = type.slice(at + 2)
    return last.startsWith('on-') ? type : `${type.slice(0, at)}::on-${last}`
  }
  const colon = type.indexOf(':')
  return colon > 0
    ? `${type.slice(0, colon)}::on-${type.slice(colon + 1)}`
    : type
}

/**
 * A stored config as the observer's args (for a function that takes them) or
 * as `$eq` filters on the event's fields (for one that does not).
 */
export function sourceOfStoredConfig(
  functionId: string,
  config: unknown,
): { args?: Record<string, unknown>; filters?: Record<string, unknown> } {
  const fields = recordOf(config)
  if (!fields || Object.keys(fields).length === 0) return {}
  if (ARG_SOURCES.has(functionId)) return { args: fields }
  const filters: Record<string, unknown> = {}
  for (const [field, value] of Object.entries(fields)) {
    if (field === 'metadata' || field === 'condition_function_id') continue
    if (value === null || typeof value === 'object') continue
    filters[`input.${field}`] = { $eq: value }
  }
  return Object.keys(filters).length > 0 ? { filters } : {}
}

/**
 * An observer activity record, read from either custom type. A record written
 * before 0.26 has its source translated so the timeline and the panel show
 * the function that replaced its event type.
 */
export function observerFiredOf(data: unknown): ObserverFiredData {
  const record = recordOf(data) ?? {}
  if (typeof record.observer_id === 'string') {
    return record as unknown as ObserverFiredData
  }
  const storedType = stringOf(record.trigger_type)
  const isState = storedType === undefined && typeof record.key === 'string'
  const functionId = storedType
    ? functionOfStoredType(storedType)
    : isState
      ? 'state::on-changed'
      : undefined
  const source = functionId
    ? sourceOfStoredConfig(
        functionId,
        record.config !== undefined
          ? record.config
          : isState
            ? { scope: record.scope, key: record.key }
            : undefined,
      )
    : {}
  const {
    subscription_id,
    trigger_id: _triggerId,
    trigger_type: _triggerType,
    config: _config,
    ...rest
  } = record
  return {
    ...(rest as unknown as Omit<ObserverFiredData, 'observer_id'>),
    observer_id: typeof subscription_id === 'string' ? subscription_id : '',
    ...(functionId ? { function_id: functionId } : {}),
    ...source,
  }
}
