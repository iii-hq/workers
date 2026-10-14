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

export interface SubAgentRow {
  usage: SessionUsage
  /** Levels below the conversation the modal is open on (children are 1). */
  depth: number
}

export interface ConversationTree {
  self: SessionUsage
  /** Every descendant, depth-first, so a grandchild follows its parent. */
  subAgents: SubAgentRow[]
  /** This conversation plus every descendant; `null` without descendants. */
  withSubAgents: UsageCounters | null
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
  return {
    self,
    subAgents,
    withSubAgents: subAgents.length
      ? sumUsage([self, ...subAgents.map((row) => row.usage)])
      : null,
  }
}

const SUMMED = [
  'input_tokens',
  'output_tokens',
  'cache_read_tokens',
  'cache_write_tokens',
  'cost_usd',
] as const

/**
 * Sum of the counters. The server reports an optional counter as `null` when
 * it cannot vouch for it (a session's generations left it unreported), so a
 * sum over such a row would be a partial total shown as a whole one: the
 * counter becomes unknown (absent) instead. A session with no steps yet has
 * nothing to report and does not count against the sum.
 */
export function sumUsage(rows: UsageCounters[]): UsageCounters {
  const total: UsageCounters = { turns: 0, function_calls: 0, function_call_errors: 0 }
  const unknown = new Set<(typeof SUMMED)[number]>()
  for (const row of rows) {
    total.turns += row.turns ?? 0
    total.function_calls += row.function_calls ?? 0
    total.function_call_errors += row.function_call_errors ?? 0
    for (const key of SUMMED) {
      const value = row[key]
      if (value != null) total[key] = (total[key] ?? 0) + value
      else if (row.turns > 0) unknown.add(key)
    }
  }
  for (const key of unknown) delete total[key]
  return total
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
