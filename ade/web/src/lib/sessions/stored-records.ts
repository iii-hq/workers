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

/**
 * The functions that replaced the engine's and the workers' event types
 * whose names the naming rule does not give (workers contract §1.1 and §3).
 */
const RENAMED_SOURCES: Readonly<Record<string, string>> = {
  cron: 'cron::on-tick',
  http: 'http::on-request',
  'durable:subscriber': 'queue::on-message',
  'database::row-changed': 'database::on-row-changed',
  state: 'state::on-changed',
  subscribe: 'publish',
  'compose-operation': 'compose::on-operation-progress',
  'console:script': 'console::ui::asset',
  'console:style': 'console::ui::asset',
  'console:module': 'console::ui::asset',
  'console:assets': 'console::ui::on-assets-changed',
  timer: 'harness::on-timer',
  'harness::triggers-changed': 'harness::on-observers-changed',
  'harness::hook::pre-trigger': 'harness::hook::on-pre-call',
  'harness::hook::post-trigger': 'harness::hook::on-post-call',
  log: 'observability::on-log',
  trace: 'observability::on-span',
  configuration: 'configuration::on-updated',
  'engine::functions-available': 'engine::on-functions-changed',
  'engine::workers-available': 'engine::on-workers-changed',
  stream: 'stream::on-changed',
  'stream:join': 'stream::on-join',
  'stream:leave': 'stream::on-leave',
}

/**
 * The functions whose observers pass args (workers contract §1.3), besides
 * the agent feeds; every other function's observers pass filters.
 */
const ARG_FUNCTIONS: ReadonlySet<string> = new Set([
  'cron::on-tick',
  'http::on-request',
  'queue::on-message',
  'database::on-row-changed',
  'harness::hook::on-pre-turn',
  'harness::hook::on-pre-generate',
  'harness::hook::on-post-generate',
  'harness::hook::on-pre-call',
  'harness::hook::on-post-call',
  'harness::hook::on-post-turn',
  'harness::on-timer',
  'console::ui::asset',
  'shell::on-changed',
  'shell::on-git-changed',
  'shell::on-job-finished',
  'github::pr::on-event',
  'storage::on-object-created',
  'storage::on-object-deleted',
  'kanban::on-change',
  'kanban::on-comment',
  'stories::on-changed',
  'computer::on-frame-changed',
  'browser::on-frame-event',
  'browser::on-crawl-item',
  'ios-simulator::on-frame-event',
  'demo::on-counter-changed',
])

/** Whether a function's observers pass args: §1.3's list and the agent feeds. */
export function takesArgs(functionId: string): boolean {
  return (
    ARG_FUNCTIONS.has(functionId) ||
    functionId.endsWith('::on-agent-event') ||
    functionId.endsWith('::on-raw-event')
  )
}

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

/** Config keys that only steered the old registration, never the event. */
const DROPPED_KEYS: ReadonlySet<string> = new Set([
  'metadata',
  'condition_function_id',
])

type Fields = Record<string, unknown>

/** The config without the keys that steered the old registration. */
function argsOf(config: Fields): Fields | undefined {
  const args = Object.fromEntries(
    Object.entries(config).filter(([key]) => !DROPPED_KEYS.has(key)),
  )
  return Object.keys(args).length > 0 ? args : undefined
}

/** One `$eq` filter per present config field, at the path given for it. */
function eqFilters(
  config: Fields,
  paths: Readonly<Record<string, string>>,
): Fields | undefined {
  const filters: Fields = {}
  for (const [field, path] of Object.entries(paths)) {
    const value = config[field]
    if (value !== undefined && value !== null) filters[path] = { $eq: value }
  }
  return Object.keys(filters).length > 0 ? filters : undefined
}

/** A `$eq` filter on `input.<field>` for each scalar config field. */
function fieldFilters(config: Fields): Fields | undefined {
  const paths: Record<string, string> = {}
  for (const [field, value] of Object.entries(config)) {
    if (DROPPED_KEYS.has(field)) continue
    if (value === null || typeof value === 'object') continue
    paths[field] = `input.${field}`
  }
  return eqFilters(config, paths)
}

/** What an observer of a stored event type names: function, args, filters. */
export interface StoredSource {
  function_id: string
  args?: Fields
  filters?: Fields
}

function sourceOf(
  function_id: string,
  args?: Fields,
  filters?: Fields,
): StoredSource {
  return {
    function_id,
    ...(args ? { args } : {}),
    ...(filters ? { filters } : {}),
  }
}

/**
 * A stored event type and its config as the source an observer names: the
 * function that replaced the type, with the config as its args (for a
 * function that takes them) or as filters on its input, as the harness
 * translates stored bindings (harness/src/bindings/migrate.rs).
 */
export function sourceOfStored(type: string, config: unknown): StoredSource {
  const fields = recordOf(config) ?? {}
  const functionId = functionOfStoredType(type)
  switch (type) {
    case 'timer':
      return sourceOf(
        functionId,
        typeof fields.at === 'number' ? { at: fields.at } : undefined,
      )
    case 'cron':
      return sourceOf(
        functionId,
        typeof fields.expression === 'string'
          ? { expression: fields.expression }
          : undefined,
      )
    case 'console:script':
    case 'console:style':
    case 'console:module':
      return sourceOf(functionId, {
        kind: type.slice('console:'.length),
        ...argsOf(fields),
      })
    case 'state':
      return sourceOf(
        functionId,
        undefined,
        eqFilters(fields, { scope: 'input.scope', key: 'input.key' }),
      )
    case 'subscribe':
      return sourceOf(
        functionId,
        undefined,
        eqFilters(fields, { topic: 'input.topic' }),
      )
    case 'stream':
    case 'stream:join':
    case 'stream:leave':
      return sourceOf(
        functionId,
        undefined,
        eqFilters(fields, {
          stream_name: 'input.stream_name',
          group_id: 'input.group_id',
        }),
      )
    case 'compose-operation': {
      const filters: Fields = {
        ...eqFilters(fields, { operation_id: 'input.operation_id' }),
        ...(fields.terminal_only === true
          ? { 'input.terminal': { $eq: true } }
          : {}),
      }
      return sourceOf(
        functionId,
        undefined,
        Object.keys(filters).length > 0 ? filters : undefined,
      )
    }
    case 'log':
      return sourceOf(
        functionId,
        undefined,
        fields.level === 'error'
          ? { 'input.severity_number': { $gte: 17 } }
          : undefined,
      )
    case 'configuration':
      return sourceOf(
        functionId,
        undefined,
        eqFilters(fields, { configuration_id: 'input.id' }),
      )
    case 'trace':
    case 'engine::functions-available':
    case 'engine::workers-available':
      return sourceOf(functionId)
    default:
      return takesArgs(functionId)
        ? sourceOf(functionId, argsOf(fields))
        : sourceOf(functionId, undefined, fieldFilters(fields))
  }
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
  const type = storedType ?? (isState ? 'state' : undefined)
  const source = type
    ? sourceOfStored(
        type,
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
    ...source,
  }
}
