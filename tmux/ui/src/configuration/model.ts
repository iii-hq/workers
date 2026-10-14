/**
 * The `tmux` configuration entry as the form reads and edits it. Nothing is
 * normalized on read: defaults are displayed, never materialized, and every
 * key the form does not know (today's or a future worker's) survives an
 * edit untouched.
 */

import type { JsonValue } from '@iii-dev/console-ui'

export type JsonObject = { [key: string]: JsonValue }

export const NUMBER_FIELDS = {
  poll_interval_ms: { label: 'Session refresh', default: 1000, min: 250, max: 60_000, unit: 'ms' },
  ports_interval_ms: { label: 'Port scan interval', default: 3000, min: 1000, max: 300_000, unit: 'ms' },
  busy_window_ms: { label: 'Working window', default: 2000, min: 250, max: 60_000, unit: 'ms' },
  command_timeout_ms: { label: 'tmux command timeout', default: 5000, min: 500, max: 60_000, unit: 'ms' },
  max_terminals: { label: 'Console terminals', default: 8, min: 1, max: 64, unit: '' },
  terminal_idle_timeout_ms: { label: 'Silent terminal timeout', default: 30_000, min: 5_000, max: 3_600_000, unit: 'ms' },
} as const

export type NumberField = keyof typeof NUMBER_FIELDS
export type TextField = 'tmux_binary' | 'socket_name' | 'socket_path'
export type PatternList = 'working_patterns' | 'waiting_patterns'

export const DEFAULT_TMUX_BINARY = 'tmux'

export interface AgentModel {
  raw: JsonObject
  id: string
  label: string
  processes: string[]
  working: string[]
  waiting: string[]
}

export interface TmuxConfigurationModel {
  value: JsonObject
  hasOpaqueRoot: boolean
  /** The `agents` key is absent: the worker's built-in matchers apply. */
  agentsDefault: boolean
  /** `agents` holds something other than a list; it is preserved as-is. */
  agentsOpaque: boolean
  agents: AgentModel[]
}

export function isObject(value: JsonValue | undefined): value is JsonObject {
  return value !== null && value !== undefined && typeof value === 'object' && !Array.isArray(value)
}

function strings(value: JsonValue | undefined): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
}

function readAgent(value: JsonValue): AgentModel {
  const raw = isObject(value) ? { ...value } : {}
  return {
    raw,
    id: typeof raw.id === 'string' ? raw.id : '',
    label: typeof raw.label === 'string' ? raw.label : '',
    processes: strings(raw.process),
    working: strings(raw.working_patterns),
    waiting: strings(raw.waiting_patterns),
  }
}

export function readTmuxConfiguration(value: JsonValue): TmuxConfigurationModel {
  const hasOpaqueRoot = !isObject(value)
  const root = isObject(value) ? { ...value } : {}
  const agentsValue = root.agents
  return {
    value: root,
    hasOpaqueRoot,
    agentsDefault: agentsValue === undefined || agentsValue === null,
    agentsOpaque: agentsValue !== undefined && agentsValue !== null && !Array.isArray(agentsValue),
    agents: Array.isArray(agentsValue) ? agentsValue.map(readAgent) : [],
  }
}

export function textValue(model: TmuxConfigurationModel, key: TextField): string {
  const value = model.value[key]
  return typeof value === 'string' ? value : ''
}

export function numberValue(model: TmuxConfigurationModel, key: NumberField): number | null {
  const value = model.value[key]
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

/** Set a text key; empty removes it so the worker default applies. */
export function withText(value: JsonValue, key: TextField, next: string): JsonValue {
  const model = readTmuxConfiguration(value)
  if (model.hasOpaqueRoot) return value
  const out: JsonObject = { ...model.value }
  if (next.trim() === '') delete out[key]
  else out[key] = next
  return out
}

/** Set a number key; `null` removes it so the worker default applies. */
export function withNumber(value: JsonValue, key: NumberField, next: number | null): JsonValue {
  const model = readTmuxConfiguration(value)
  if (model.hasOpaqueRoot) return value
  const out: JsonObject = { ...model.value }
  if (next === null) delete out[key]
  else out[key] = next
  return out
}

/**
 * What a typed number means: `null` for empty (use the default), a whole
 * number, or `undefined` while the text is not a number yet (nothing is
 * committed then, so `NaN` never reaches the draft).
 */
export function parseWholeNumber(text: string): number | null | undefined {
  const trimmed = text.trim()
  if (trimmed === '') return null
  if (!/^\d+$/.test(trimmed)) return undefined
  const parsed = Number(trimmed)
  return Number.isSafeInteger(parsed) ? parsed : undefined
}

export function numberError(key: NumberField, value: number | null): string | null {
  if (value === null) return null
  const spec = NUMBER_FIELDS[key]
  if (value < spec.min || value > spec.max) return `Between ${spec.min} and ${spec.max}${spec.unit ? ` ${spec.unit}` : ''}.`
  return null
}

function writeAgents(model: TmuxConfigurationModel, agents: JsonObject[]): JsonValue {
  return { ...model.value, agents }
}

function agentJson(agent: AgentModel): JsonObject {
  return {
    ...agent.raw,
    id: agent.id,
    label: agent.label,
    process: agent.processes,
    working_patterns: agent.working,
    waiting_patterns: agent.waiting,
  }
}

export function withAgent(value: JsonValue, index: number, patch: Partial<Omit<AgentModel, 'raw'>>): JsonValue {
  const model = readTmuxConfiguration(value)
  if (model.hasOpaqueRoot || model.agentsOpaque || !model.agents[index]) return value
  const agents = model.agents.map((agent, i) => agentJson(i === index ? { ...agent, ...patch } : agent))
  return writeAgents(model, agents)
}

/** Append an agent. On a default (absent) list this starts an explicit one. */
export function withNewAgent(value: JsonValue, id: string): JsonValue {
  const model = readTmuxConfiguration(value)
  if (model.hasOpaqueRoot || model.agentsOpaque) return value
  const agents = model.agents.map(agentJson)
  agents.push(agentJson({ raw: {}, id, label: id, processes: [id], working: [], waiting: [] }))
  return writeAgents(model, agents)
}

export function withoutAgent(value: JsonValue, index: number): JsonValue {
  const model = readTmuxConfiguration(value)
  if (model.hasOpaqueRoot || model.agentsOpaque || !model.agents[index]) return value
  return writeAgents(
    model,
    model.agents.filter((_, i) => i !== index).map(agentJson),
  )
}

/** Drop an explicit list so the worker's built-in matchers apply again. */
export function withDefaultAgents(value: JsonValue): JsonValue {
  const model = readTmuxConfiguration(value)
  if (model.hasOpaqueRoot) return value
  const out: JsonObject = { ...model.value }
  delete out.agents
  return out
}

/** Comma- or whitespace-separated process names. */
export function parseProcesses(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((item) => item.trim())
    .filter(Boolean)
}

export function patternError(pattern: string): string | null {
  if (pattern === '') return 'Empty patterns match everything; remove it instead.'
  try {
    new RegExp(pattern)
    return null
  } catch {
    return 'Not a valid regular expression.'
  }
}

export function agentIdError(agents: readonly AgentModel[], index: number): string | null {
  const id = agents[index]?.id ?? ''
  if (!/^[a-z0-9][a-z0-9_-]{0,31}$/.test(id)) return 'Lowercase letters, digits, - and _ (up to 32).'
  if (agents.some((agent, i) => i !== index && agent.id === id)) return 'Another agent uses this id.'
  return null
}
