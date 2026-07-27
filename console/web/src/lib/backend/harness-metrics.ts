/**
 * Typed wrapper over `harness::metrics` — the only source of usage totals for
 * a whole *session tree*, i.e. including sub-agent sessions spawned by
 * `harness::spawn`, plus trace/span counts. Shapes mirror
 * `harness/src/functions/metrics.rs:19-106`.
 *
 * Two properties make this a bonus rather than a primary source, and both are
 * why the console computes its own rollup from the transcript instead:
 *
 * - It returns everything ZEROED with `complete: false` unless every session
 *   in the tree has a terminal turn record (`metrics.rs:140-147`) — so it is
 *   unavailable exactly while a chat is active.
 * - Its token fields are all-or-nothing sums: one generation missing a field
 *   collapses that field to `null` for the entire tree.
 *
 * We therefore discard an incomplete payload entirely rather than render a
 * confident, wrong `$0.000000`.
 */

import { getIiiClient } from '@/lib/iii-client'

const METRICS_FN = 'harness::metrics'
const TIMEOUT_MS = 10_000

export interface SessionUsageTotalsV1 {
  sessions: number
  /**
   * Assistant messages, i.e. model calls — `metrics.rs:305` increments this
   * once per assistant message, so a turn with three tool rounds counts as
   * three. The console labels this "steps" and keeps "turns" for real turns.
   */
  turns: number
  function_calls: number
  function_call_errors: number
  input_tokens?: number | null
  output_tokens?: number | null
  cache_read_tokens?: number | null
  cache_write_tokens?: number | null
  reasoning_tokens?: number | null
  cost_usd?: number | null
}

export interface SessionUsageV1 extends SessionUsageTotalsV1 {
  session_id: string
  parent_session_id?: string | null
  depth: number
}

export interface SessionTraceMetricsV1 {
  trace_count: number
  span_count: number
  error_span_count: number
  duration_ms: number
}

export interface SessionMetricsResponseV1 {
  root_session_id: string
  complete: boolean
  totals: SessionUsageTotalsV1
  by_session: SessionUsageV1[]
  traces?: SessionTraceMetricsV1 | null
}

export type HarnessMetricsState =
  | { status: 'ok'; metrics: SessionMetricsResponseV1 }
  /** The tree has a turn in flight — totals would be all zeros. */
  | { status: 'incomplete' }
  /** The harness is absent, errored, or timed out. */
  | { status: 'unavailable' }

export async function fetchHarnessMetrics(
  rootSessionId: string,
): Promise<HarnessMetricsState> {
  try {
    const client = await getIiiClient()
    const raw = (await client.trigger(
      METRICS_FN,
      { root_session_id: rootSessionId },
      { timeoutMs: TIMEOUT_MS },
    )) as SessionMetricsResponseV1 | null

    if (!raw || typeof raw !== 'object') return { status: 'unavailable' }
    // An incomplete response is not partial data — it is all zeros.
    if (!raw.complete) return { status: 'incomplete' }
    return { status: 'ok', metrics: raw }
  } catch {
    return { status: 'unavailable' }
  }
}
