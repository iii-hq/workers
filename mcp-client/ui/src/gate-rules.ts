import type { FunctionAccess } from './access'
import type { Tone } from './presentation'
import type { Server } from './types'

/** The approval-gate rules this page writes (spec 5.6): one exact allow rule
 * per read function, grouped per server in a block marked by its `rule_id`.
 * Every other item of the gate's `rules` belongs to someone else and comes
 * back as the same value, in its place. */

export const RULE_ID_PREFIX = 'mcp-client:'

export interface GateRule {
  function: string
  action: 'allow'
  rule_id: string
}

export function ruleFor(functionId: string): GateRule {
  return { function: functionId, action: 'allow', rule_id: `${RULE_ID_PREFIX}${functionId}` }
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value)

// The trailing `::` keeps `linear` out of `linear-app`.
const inBlock = (item: unknown, server: string): item is Record<string, unknown> =>
  isObject(item) && typeof item.rule_id === 'string' && item.rule_id.startsWith(`${RULE_ID_PREFIX}mcp::${server}::`)

export function blockOf(rules: unknown[], server: string): { ids: string[]; firstIndex: number } {
  return {
    // String(): a hand-edited `function` that is not a string must not throw further on.
    ids: rules.filter((item) => inBlock(item, server)).map((item) => String(item.function)),
    firstIndex: rules.findIndex((item) => inBlock(item, server)),
  }
}

/** Matches every function for every call: a catch-all decides before any rule after it. */
const isCatchAll = (item: unknown) =>
  item === '*' || item === '!*' || (isObject(item) && item.function === '*' && !('modes' in item || 'args' in item))

export function mergeGateRules(rules: unknown[], server: string, readIds: string[]): unknown[] {
  const own = readIds.filter((id) => id.startsWith(`mcp::${server}::`) && !id.includes('*'))
  const block = [...new Set(own)].sort().map(ruleFor)
  const rest = rules.filter((item) => !inBlock(item, server))
  // No block rule comes before the first one, so its index counts foreign items only.
  const first = blockOf(rules, server).firstIndex
  const catchAll = rest.findIndex(isCatchAll)
  const at = first >= 0 ? first : catchAll >= 0 ? catchAll : rest.length
  return [...rest.slice(0, at), ...block, ...rest.slice(at)]
}

/** The gate's entry has `deny_unknown_fields`: every key comes back as read, none is added. */
export function mergeGateEntry(
  value: unknown,
  server: string,
  readIds: string[],
): { ok: true; value: Record<string, unknown> } | { ok: false; reason: string } {
  if (!isObject(value)) return { ok: false, reason: 'The approval-gate entry is not an object.' }
  if (!Array.isArray(value.rules)) return { ok: false, reason: 'The approval-gate entry has no rules array.' }
  return { ok: true, value: { ...value, rules: mergeGateRules(value.rules, server, readIds) } }
}

const BLOCK_SERVER = /^mcp-client:mcp::(.*?)::/

export function orphanBlocks(rules: unknown[], configured: string[]): { server: string; count: number }[] {
  const counts = new Map<string, number>()
  for (const item of rules) {
    const server = isObject(item) && typeof item.rule_id === 'string' ? BLOCK_SERVER.exec(item.rule_id)?.[1] : undefined
    if (server !== undefined && !configured.includes(server)) counts.set(server, (counts.get(server) ?? 0) + 1)
  }
  return [...counts.keys()].sort().map((server) => ({ server, count: counts.get(server) ?? 0 }))
}

export type GateKind =
  | 'no_gate'
  | 'not_connected'
  | 'no_rules'
  | 'present'
  | 'missing'
  | 'stale_harmless'
  | 'stale_unsafe'
  | 'overridden_read'
  | 'overridden_write'
  | 'unreadable'

/** The reply of `approval::evaluate` for one function. */
export interface Verdict {
  verdict: 'allow' | 'deny' | 'needs_approval'
  reason?: string
}

export interface GateStatus {
  kind: GateKind
  /** Every status that applies, most severe first. */
  all: GateKind[]
  tone: Tone
  rowText: string
  toAdd: string[]
  toRemove: string[]
  unsafe: string[]
  overridden: { functionId: string; verdict: Verdict }[]
  allowed: number
  ask: number
  rowAction: boolean
}

const SEVERITY: GateKind[] = [
  'stale_unsafe',
  'overridden_write',
  'unreadable',
  'missing',
  'stale_harmless',
  'overridden_read',
  'present',
  'no_rules',
]

const TONE: Record<GateKind, Tone> = {
  stale_unsafe: 'alert',
  overridden_write: 'alert',
  unreadable: 'alert',
  missing: 'warn',
  stale_harmless: 'warn',
  overridden_read: 'warn',
  no_gate: 'warn',
  present: 'ok',
  no_rules: 'ghost',
  not_connected: 'ghost',
}

export function gateStatus(input: {
  gate: { installed: boolean; rules: unknown[] | null; error?: string }
  server: Server
  access: FunctionAccess[]
  verdicts?: Record<string, Verdict>
}): GateStatus {
  const { gate, server, access, verdicts = {} } = input
  // A rule copied by hand counts once.
  const block = [...new Set(blockOf(gate.rules ?? [], server.name).ids)]
  const reads = access.filter((fn) => fn.cls === 'read').map((fn) => fn.functionId)
  const early: GateKind | null = !gate.installed
    ? 'no_gate'
    : gate.rules === null
      ? 'unreadable'
      : server.tools_listed
        ? null
        : 'not_connected'

  const toAdd = early ? [] : reads.filter((id) => !block.includes(id)).sort()
  const toRemove = early ? [] : block.filter((id) => !reads.includes(id)).sort()
  const isWrite = (id: string) => access.some((fn) => fn.functionId === id && fn.cls !== 'read')
  // Not an exact id of this server (a glob, a foreign or a namespaced id): the page never meant to allow it.
  const unsafe = toRemove.filter((id) => isWrite(id) || id.includes('*') || !id.startsWith(`mcp::${server.name}::`))
  const harmless = toRemove.filter((id) => !unsafe.includes(id))
  // A function with no verdict (no probe yet, or a failed one) is never overridden.
  const overridden = early
    ? []
    : access.filter((fn) =>
        fn.cls === 'read'
          ? block.includes(fn.functionId) && (verdicts[fn.functionId]?.verdict ?? 'allow') !== 'allow'
          : !unsafe.includes(fn.functionId) && verdicts[fn.functionId]?.verdict === 'allow',
      )

  const applies: Partial<Record<GateKind, boolean>> = {
    stale_unsafe: unsafe.length > 0,
    overridden_write: overridden.some((fn) => fn.cls !== 'read'),
    missing: block.length > 0 && toAdd.length > 0,
    stale_harmless: harmless.length > 0,
    overridden_read: overridden.some((fn) => fn.cls === 'read'),
    present: block.length > 0 && toAdd.length === 0 && toRemove.length === 0,
    no_rules: block.length === 0,
  }
  const all = early ? [early] : SEVERITY.filter((kind) => applies[kind])
  const kind = all[0]
  const allowed = early === 'not_connected' ? block.length : reads.filter((id) => block.includes(id)).length
  const rowText: Record<GateKind, string> = {
    no_gate: 'No approval gate · writes open',
    not_connected: 'Connect to check',
    no_rules: 'Every call asks',
    present: 'Reads allowed · writes ask',
    missing: `${toAdd.length} new read ${toAdd.length === 1 ? 'tool still asks' : 'tools still ask'}`,
    stale_harmless: `${harmless.length} ${harmless.length === 1 ? 'rule for a removed tool' : 'rules for removed tools'}`,
    stale_unsafe: `${unsafe.length} ${unsafe.length === 1 ? 'write' : 'writes'} allowed without asking`,
    overridden_read: 'Overridden in the approval gate',
    overridden_write: 'Overridden in the approval gate',
    unreadable: 'Cannot read the approval-gate rules',
  }

  return {
    kind,
    all,
    tone: TONE[kind],
    rowText: rowText[kind],
    toAdd,
    toRemove,
    unsafe,
    overridden: overridden.map((fn) => ({ functionId: fn.functionId, verdict: verdicts[fn.functionId] })),
    allowed,
    ask: early === 'not_connected' ? 0 : access.length - allowed,
    rowAction: server.state === 'ready' && (kind === 'stale_unsafe' || kind === 'overridden_write'),
  }
}
