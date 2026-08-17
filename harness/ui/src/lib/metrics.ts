/**
 * Wire shapes of `harness::metrics` (SessionMetricsResponseV1) and the
 * per-generation context snapshot (ContextSnapshotV1) it carries, plus the
 * tolerant parsers the chip and the function-trigger renderer share. A
 * snapshot from a newer harness must degrade to "no data", never a crash.
 */

export interface SnapshotMessages {
  user: number
  assistant: number
  function_result: number
  custom: number
}

export interface SnapshotCategories {
  system_prompt: number
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

/** `{ content: [...], details }` harness result envelope → details. */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
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
