/**
 * Engine transport for the traces page.
 *
 * Mirrors `motia/console/.../api/observability/traces.ts`, but resolves
 * the iii-browser-sdk client from the shared `getIiiClient()` singleton
 * instead of taking the SDK as a parameter. Engine RPC paths are
 * identical, so the engine contract is unchanged.
 *
 * Timeout note: motia passes per-RPC timeouts to `sdk.trigger`. Our
 * `IiiClient.trigger` wrapper does not yet expose `timeoutMs`, so calls
 * here use the SDK's default invocation timeout (30s). If this becomes
 * a latency problem (long list requests, slow tree requests), extend
 * the wrapper rather than reaching past it.
 */

import { getIiiClient } from '@/lib/iii-client'

export interface SpanEvent {
  name: string
  timestamp_unix_nano: number
  attributes: Record<string, unknown>
}

export interface SpanLink {
  trace_id: string
  span_id: string
  attributes: Record<string, unknown>
}

export interface StoredSpan {
  trace_id: string
  span_id: string
  parent_span_id?: string
  name: string
  kind?: string
  start_time_unix_nano: number
  end_time_unix_nano: number
  status: string
  attributes: Array<[string, unknown]>
  events: SpanEvent[]
  links: SpanLink[]
  flags?: number
  service_name?: string
  resource?: Record<string, unknown>
  /** In-flight live snapshot: still running, end_time_unix_nano is 0. */
  pending?: boolean
  /** Trace-level tags (`iii.tag.*` + session/message identity attributes)
   *  merged across the trace by `engine::traces::list`. Only present on
   *  list rows; live-streamed rows don't carry it. */
  trace_tags?: Record<string, string>
}

export interface TracesResponse {
  spans: StoredSpan[]
  total: number
  offset: number
  limit: number
}

export interface TracesFilterParams {
  trace_id?: string
  service_name?: string
  name?: string
  status?: 'ok' | 'error' | 'unset'
  span_id?: string
  parent_span_id?: string | null
  min_duration_ms?: number
  max_duration_ms?: number
  start_time?: number
  end_time?: number
  attributes?: [string, string][]
  sort_by?: 'start_time' | 'duration' | 'service_name'
  sort_order?: 'asc' | 'desc'
  offset?: number
  limit?: number
  include_internal?: boolean
  search_all_spans?: boolean
}

export interface SpanTreeNode {
  trace_id: string
  span_id: string
  parent_span_id?: string
  name: string
  kind?: string
  start_time_unix_nano: number
  end_time_unix_nano: number
  status: string
  attributes: Array<[string, unknown]>
  events: SpanEvent[]
  links: SpanLink[]
  flags?: number
  service_name?: string
  resource?: Record<string, unknown>
  /** In-flight live snapshot: still running, end_time_unix_nano is 0. */
  pending?: boolean
  children: SpanTreeNode[]
}

export interface TraceTreeResponse {
  roots: SpanTreeNode[]
}

export interface TracesGroupByParams {
  attribute: string
  since_ms?: number
  limit?: number
  include_internal?: boolean
}

export interface TraceGroup {
  value: string
  trace_ids: string[]
  span_count: number
  first_seen_ms: number
  last_seen_ms: number
  duration_ms: number
  error_count: number
}

export interface TracesGroupByResponse {
  groups: TraceGroup[]
}

export const TRACES_RPC_FUNCTIONS = {
  list: 'engine::traces::list',
  tree: 'engine::traces::tree',
  clear: 'engine::traces::clear',
  groupBy: 'engine::traces::group_by',
} as const

function stripUndefined<T extends Record<string, unknown>>(obj: T): T {
  return Object.fromEntries(
    Object.entries(obj).filter(([, v]) => v !== undefined),
  ) as T
}

function isMemoryExporterNotEnabled(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err)
  return /memory exporter (is )?not enabled/i.test(msg)
}

function asError(err: unknown, fallback: string): Error {
  if (err instanceof Error) return err
  return new Error(typeof err === 'string' ? err : fallback)
}

export async function fetchTraces(
  options?: TracesFilterParams,
): Promise<TracesResponse> {
  const offset = options?.offset ?? 0
  const limit = options?.limit ?? 100
  const payload = stripUndefined({ ...options, offset, limit })

  try {
    const client = await getIiiClient()
    return await client.trigger<TracesResponse>(
      TRACES_RPC_FUNCTIONS.list,
      payload,
    )
  } catch (err) {
    if (isMemoryExporterNotEnabled(err)) {
      return { spans: [], total: 0, offset, limit }
    }
    throw asError(err, 'Failed to fetch traces')
  }
}

export async function fetchTraceTree(
  traceId: string,
): Promise<TraceTreeResponse> {
  try {
    const client = await getIiiClient()
    return await client.trigger<TraceTreeResponse>(TRACES_RPC_FUNCTIONS.tree, {
      trace_id: traceId,
    })
  } catch (err) {
    if (isMemoryExporterNotEnabled(err)) {
      return { roots: [] }
    }
    throw asError(err, 'Failed to fetch trace tree')
  }
}

export async function clearTraces(): Promise<{ success: boolean }> {
  try {
    const client = await getIiiClient()
    await client.trigger(TRACES_RPC_FUNCTIONS.clear, {})
    return { success: true }
  } catch (err) {
    throw asError(err, 'Failed to clear traces')
  }
}

export async function fetchTracesGroupBy(
  params: TracesGroupByParams,
): Promise<TracesGroupByResponse> {
  const payload = stripUndefined({ ...params })
  try {
    const client = await getIiiClient()
    return await client.trigger<TracesGroupByResponse>(
      TRACES_RPC_FUNCTIONS.groupBy,
      payload,
    )
  } catch (err) {
    if (isMemoryExporterNotEnabled(err)) {
      return { groups: [] }
    }
    throw asError(err, 'Failed to fetch trace groups')
  }
}
