/**
 * Wire shapes of `harness::metrics` (SessionMetricsResponseV1) and the
 * per-generation context snapshot (ContextSnapshotV1) it carries, plus the
 * tolerant parsers the chip and the function-trigger renderer share. A
 * snapshot from a newer harness must degrade to "no data", never a crash.
 */

import { unwrapEnvelope } from '@iii-dev/console-ui/format'

export interface SnapshotMessages {
  user: number
  assistant: number
  function_result: number
  custom: number
}

export interface SnapshotCategories {
  system_prompt: number
  /** Optional on the wire: absent in snapshots written before Skills. */
  skills?: number
  tools: number
  messages: SnapshotMessages
  overhead: number
  /** `#[serde(default)]` on the Rust side — absent in snapshots written
      before the category existed. */
  hook_guidance?: number
}

export interface SnapshotUsage {
  input?: number
  output?: number
  cache_read?: number
  cache_write?: number
  reasoning?: number
  cost_usd?: number
}

export interface ContextSnapshot {
  session_id: string
  turn_id: string
  step: number
  model: string
  provider?: string
  estimator?: string
  usable: number
  effective_max_output_tokens: number
  total: number
  free: number
  categories: SnapshotCategories
  compacted: boolean
  summarized_head_tokens?: number
  usage?: SnapshotUsage
  /** Running cost of the whole session, summed across generation steps. */
  session_cost_usd?: number | null
  timestamp: number
}

export interface SessionUsage {
  session_id: string
  parent_session_id?: string
  depth: number
  turns: number
  function_calls: number
  function_call_errors: number
  input_tokens?: number
  output_tokens?: number
  cache_read_tokens?: number
  cache_write_tokens?: number
  reasoning_tokens?: number
  cost_usd?: number
  context?: ContextSnapshot
}

export interface MetricsTotals {
  sessions: number
  turns: number
  function_calls: number
  function_call_errors: number
  input_tokens?: number
  output_tokens?: number
  cache_read_tokens?: number
  cache_write_tokens?: number
  reasoning_tokens?: number
  cost_usd?: number
}

export interface MetricsResponse {
  root_session_id?: string
  complete?: boolean
  totals: MetricsTotals
  by_session: SessionUsage[]
}

export function parseMetrics(value: unknown): MetricsResponse | null {
  const raw = unwrapEnvelope(value)
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const obj = raw as Record<string, unknown>
  if (!Array.isArray(obj.by_session)) return null
  if (!obj.totals || typeof obj.totals !== 'object' || Array.isArray(obj.totals)) {
    return null
  }
  return obj as unknown as MetricsResponse
}

export function isSnapshot(value: unknown): value is ContextSnapshot {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const snap = value as Record<string, unknown>
  if (typeof snap.total !== 'number' || typeof snap.usable !== 'number')
    return false
  const categories = snap.categories as Record<string, unknown> | undefined
  if (!categories || typeof categories !== 'object' || Array.isArray(categories))
    return false
  const messages = categories.messages as Record<string, unknown> | undefined
  return (
    !!messages &&
    typeof messages === 'object' &&
    !Array.isArray(messages) &&
    typeof messages.user === 'number' &&
    typeof messages.assistant === 'number' &&
    typeof messages.function_result === 'number' &&
    typeof messages.custom === 'number'
  )
}

/** Usage counters shared by one session and a sum of several. */
export type UsageCounters = Pick<
  SessionUsage,
  | 'turns'
  | 'function_calls'
  | 'function_call_errors'
  | 'input_tokens'
  | 'output_tokens'
  | 'cache_read_tokens'
  | 'cache_write_tokens'
  | 'cost_usd'
>

const SUMMED = [
  'input_tokens',
  'output_tokens',
  'cache_read_tokens',
  'cache_write_tokens',
  'cost_usd',
] as const

export type SummedKey = (typeof SUMMED)[number]

/** A sum over several sessions. */
export interface UsageTotals extends UsageCounters {
  /** Counters some session with steps left unreported (`null` on the wire):
      the value sums the sessions that did report it, so it is a lower bound
      and reads `≥`. */
  partial: SummedKey[]
  /** The sessions behind `partial`. */
  unreported: string[]
  /** Cache hit over the sessions that reported both fresh input and cache
      read — a session missing one of them would skew the ratio, one missing
      both just is not in it. `null` when no session reported both. */
  cacheHit: number | null
}

export interface SubAgentRow {
  usage: SessionUsage
  /** Levels below the conversation the modal is open on (children are 1). */
  depth: number
}

export interface ConversationTree {
  self: SessionUsage
  /** Every descendant, depth-first, so a grandchild follows its parent. */
  subAgents: SubAgentRow[]
}

/**
 * `harness::metrics` walks a session up to the ROOT of its tree and reports
 * the whole tree. The modal is about one conversation: this cuts out its own
 * subtree (itself and its descendants, by `parent_session_id`). `null` when
 * the response does not carry the session at all.
 */
export function conversationTree(
  metrics: MetricsResponse,
  sessionId: string,
): ConversationTree | null {
  const self = metrics.by_session.find((row) => row.session_id === sessionId)
  if (!self) return null
  const children = new Map<string, SessionUsage[]>()
  for (const row of metrics.by_session) {
    if (!row.parent_session_id) continue
    const list = children.get(row.parent_session_id) ?? []
    list.push(row)
    children.set(row.parent_session_id, list)
  }
  const subAgents: SubAgentRow[] = []
  // ponytail: a cycle in parent links would loop forever; `seen` stops it.
  const seen = new Set([sessionId])
  const walk = (id: string, depth: number) => {
    for (const child of children.get(id) ?? []) {
      if (seen.has(child.session_id)) continue
      seen.add(child.session_id)
      subAgents.push({ usage: child, depth })
      walk(child.session_id, depth + 1)
    }
  }
  walk(sessionId, 1)
  return { self, subAgents }
}

/**
 * Sum of the counters. A counter some session with steps left unreported
 * still sums what the others reported — with 175 sub-agents, two unpriced
 * ones must not blank the total — and is listed in `partial`. A counter no
 * session reported stays absent. A session with no steps yet has nothing to
 * report and counts against nothing.
 */
export function sumUsage(rows: SessionUsage[]): UsageTotals {
  const total: UsageTotals = {
    turns: 0,
    function_calls: 0,
    function_call_errors: 0,
    partial: [],
    unreported: [],
    cacheHit: null,
  }
  const missing: [string, SummedKey[]][] = []
  let hitRead = 0
  let hitPrompt = 0
  for (const row of rows) {
    total.turns += row.turns ?? 0
    total.function_calls += row.function_calls ?? 0
    total.function_call_errors += row.function_call_errors ?? 0
    const gaps: SummedKey[] = []
    for (const key of SUMMED) {
      const value = row[key]
      if (value != null) total[key] = (total[key] ?? 0) + value
      else if (row.turns > 0) gaps.push(key)
    }
    if (gaps.length) missing.push([row.session_id, gaps])
    if (row.input_tokens != null && row.cache_read_tokens != null) {
      hitRead += row.cache_read_tokens
      hitPrompt += row.input_tokens + row.cache_read_tokens + (row.cache_write_tokens ?? 0)
    }
  }
  total.cacheHit = hitPrompt > 0 ? Math.round((hitRead / hitPrompt) * 100) : null
  // A counter nobody reported stays absent (not a partial zero) — and a
  // session missing only such counters (cache write on a provider that never
  // reports it) is not one the `≥` leaves out.
  total.partial = SUMMED.filter(
    (key) => total[key] != null && missing.some(([, gaps]) => gaps.includes(key)),
  )
  total.unreported = missing
    .filter(([, gaps]) => gaps.some((key) => total.partial.includes(key)))
    .map(([id]) => id)
  return total
}

/**
 * The conversation's own counters with its cost taken from the live snapshot
 * when there is one (it moves every generate step; `harness::metrics` is read
 * once per turn), and the same sum over its whole subtree — `null` without
 * sub-agents.
 */
export function treeTotals(
  tree: ConversationTree,
  liveCost?: number | null,
): { self: SessionUsage; total: UsageTotals | null } {
  const self = liveCost == null ? tree.self : { ...tree.self, cost_usd: liveCost }
  const total = tree.subAgents.length
    ? sumUsage([self, ...tree.subAgents.map((row) => row.usage)])
    : null
  return { self, total }
}

/** One direct child of the conversation with everything it spawned. */
export interface SubAgentGroup {
  head: SessionUsage
  /** `head` first, then its descendants depth-first (after a filter, only
      the ones that matched). */
  rows: SessionUsage[]
  /** How many descendants the group has, filtered or not. */
  size: number
  totals: UsageTotals
  /** Newest context snapshot in the group (ms), when any session has one. */
  lastActive?: number
}

/** Groups the subtree by the conversation's direct children — with a big
    fan-out those are the orchestrators, and the rows under each are their
    agents. */
export function subAgentGroups(tree: ConversationTree): SubAgentGroup[] {
  const groups: SubAgentGroup[] = []
  let current: SessionUsage[] | null = null
  const lists: SessionUsage[][] = []
  for (const row of tree.subAgents) {
    if (row.depth === 1 || !current) {
      current = []
      lists.push(current)
    }
    current.push(row.usage)
  }
  for (const rows of lists) {
    const stamps = rows
      .map((row) => row.context?.timestamp)
      .filter((stamp): stamp is number => typeof stamp === 'number')
    groups.push({
      head: rows[0],
      rows,
      size: rows.length - 1,
      totals: sumUsage(rows),
      lastActive: stamps.length ? Math.max(...stamps) : undefined,
    })
  }
  return groups
}

export type GroupSort = 'cost' | 'recent' | 'failed'

const costOf = (usage: { cost_usd?: number }) => usage.cost_usd ?? 0
const byCost = (a: SubAgentGroup, b: SubAgentGroup) => costOf(b.totals) - costOf(a.totals)

/** A sorted copy: by subtree cost, by the newest snapshot, or by failed
    calls (cost breaks ties). */
export function sortGroups(groups: SubAgentGroup[], by: GroupSort): SubAgentGroup[] {
  const sorted = [...groups]
  if (by === 'cost') return sorted.sort(byCost)
  if (by === 'recent') return sorted.sort((a, b) => (b.lastActive ?? 0) - (a.lastActive ?? 0) || byCost(a, b))
  return sorted.sort(
    (a, b) => b.totals.function_call_errors - a.totals.function_call_errors || byCost(a, b),
  )
}

/**
 * Groups whose head or any row matches `query` (session id substring, case
 * insensitive). A group matched by its head keeps every row; otherwise it
 * keeps its head for context plus the rows that match.
 */
export function filterGroups(groups: SubAgentGroup[], query: string): SubAgentGroup[] {
  const q = query.trim().toLowerCase()
  if (!q) return groups
  const hit = (row: SessionUsage) => row.session_id.toLowerCase().includes(q)
  const out: SubAgentGroup[] = []
  for (const group of groups) {
    if (hit(group.head)) {
      out.push(group)
      continue
    }
    const rows = group.rows.slice(1).filter(hit)
    if (rows.length) out.push({ ...group, rows: [group.head, ...rows] })
  }
  return out
}

/** Cached share of the whole prompt (fresh input + cache read + cache write),
    rounded to a percent; `null` when the counts it needs are unknown. */
export function cacheHitPct(usage: UsageCounters): number | null {
  // Without the fresh-input or cache-read count the ratio is a guess.
  if (usage.input_tokens == null || usage.cache_read_tokens == null) return null
  const read = usage.cache_read_tokens
  const prompt = usage.input_tokens + read + (usage.cache_write_tokens ?? 0)
  return prompt > 0 ? Math.round((read / prompt) * 100) : null
}
