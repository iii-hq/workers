/**
 * Engine transport for the traces page.
 *
 * Mirrors `motia/console/.../api/observability/traces.ts`, but resolves
 * the iii-browser-sdk client from the shared `getIiiClient()` singleton
 * instead of taking the SDK as a parameter. The transport remains shared,
 * while list summaries and full-span detail use separate RPC contracts.
 *
 * Timeout note: motia passes per-RPC timeouts to `sdk.trigger`. Our
 * `IiiClient.trigger` wrapper does not yet expose `timeoutMs`, so calls
 * here use the SDK's default invocation timeout (30s). If this becomes
 * a latency problem (long list requests, slow tree requests), extend
 * the wrapper rather than reaching past it. The list and detail contracts
 * intentionally differ: list returns compact trace summaries, while spans
 * returns complete stored records.
 */

import { errText } from '@/lib/errors'
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
  /** In-flight live snapshot: still running, `end_time_unix_nano` is 0.
   *  Replaced by the final span (same span_id) when it closes. Only
   *  serialized when true. */
  pending?: boolean
  /** Trace-level tags (`iii.tag.*` + session/message identity attributes)
   *  merged from every span of the trace by `engine::traces::spans`. */
  trace_tags?: Record<string, string>
}

export interface TraceSummary {
  trace_id: string
  name: string
  start_time_unix_nano: number
  end_time_unix_nano?: number
  status: 'ok' | 'error' | 'pending'
  service_name?: string
  function_id?: string
  topic?: string
  trace_tags?: Record<string, string>
  /** Only keys requested through `attribute_projection` are present. */
  attributes?: Record<string, string>
  span_count: number
  error_count: number
}

export interface TracesResponse {
  traces: TraceSummary[]
  total: number
  offset: number
  limit: number
  /** Client-side marker: the engine answered "memory exporter not enabled".
   *  Set only by `fetchTraces`, never by the wire — it is what separates
   *  "observability is off" from an ordinary empty result, so the UI can
   *  reserve its no-observability message for the real thing. */
  memoryExporterDisabled?: true
}

export interface TraceSpansResponse {
  spans: StoredSpan[]
  total: number
  offset: number
  limit: number
  /** Client-side marker: the engine answered "memory exporter not enabled".
   *  Set only by `fetchTraceSpans`, never by the wire — it is what separates
   *  "observability is off" from an ordinary empty result, so the UI can
   *  reserve its no-observability message for the real thing. */
  memoryExporterDisabled?: true
}

type TracesWireResponse = TracesResponse | TraceSpansResponse

export interface TracesFilterParams {
  trace_id?: string
  /** Fetch a specific set of traces (grouped-view member expansion).
   *  Ignored when `trace_id` is set. */
  trace_ids?: string[]
  service_name?: string
  name?: string
  status?: 'ok' | 'error' | 'pending' | 'unset'
  span_id?: string
  parent_span_id?: string | null
  min_duration_ms?: number
  max_duration_ms?: number
  start_time?: number
  end_time?: number
  attributes?: [string, string][]
  /** Exclude rows whose OWN attributes match any [key, value] pair (the
   *  engine-side arm of hidden functions; the flat list filters
   *  client-side instead to keep the live-append path). */
  exclude_attributes?: [string, string][]
  sort_by?: 'start_time' | 'duration' | 'service_name'
  sort_order?: 'asc' | 'desc'
  offset?: number
  limit?: number
  include_internal?: boolean
  search_all_spans?: boolean
  /** Arbitrary attributes needed by the current list view. */
  attribute_projection?: string[]
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
  /** See StoredSpan.pending. */
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
  /** Attribute whose value becomes each group's human-readable `label`
   *  (e.g. group by `iii.session.id`, label by `iii.session.name`). */
  label_attribute?: string
}

export interface TraceGroup {
  value: string
  /** Resolved from `label_attribute` when requested and present. */
  label?: string | null
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
  spans: 'engine::traces::spans',
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
  return /memory exporter (is )?not enabled/i.test(errText(err))
}

function isFunctionUnavailable(err: unknown): boolean {
  return /function_not_found|function .*not (?:found|registered)|not registered/i.test(
    errText(err),
  )
}

function spanAttributes(span: StoredSpan): Record<string, unknown> {
  if (Array.isArray(span.attributes)) {
    return Object.fromEntries(span.attributes)
  }
  if (span.attributes && typeof span.attributes === 'object') {
    return span.attributes as unknown as Record<string, unknown>
  }
  return {}
}

function summarizeLegacySpans(
  spans: StoredSpan[],
  attributeProjection: string[] = [],
): TraceSummary[] {
  const byTrace = new Map<string, StoredSpan[]>()
  for (const span of spans) {
    const trace = byTrace.get(span.trace_id)
    if (trace) trace.push(span)
    else byTrace.set(span.trace_id, [span])
  }

  return [...byTrace.values()].flatMap((traceSpans) => {
    const ordered = [...traceSpans].sort(
      (a, b) =>
        a.start_time_unix_nano - b.start_time_unix_nano ||
        a.span_id.localeCompare(b.span_id),
    )
    const presentSpanIds = new Set(ordered.map((span) => span.span_id))
    const representative =
      ordered.find(
        (span) =>
          !span.parent_span_id || !presentSpanIds.has(span.parent_span_id),
      ) ?? ordered[0]
    if (!representative) return []

    const traceTags: Record<string, string> = {}
    const projectedAttributes: Record<string, string> = {}
    for (const span of ordered) {
      Object.assign(traceTags, span.trace_tags)
      for (const [key, rawValue] of Object.entries(spanAttributes(span))) {
        const value = String(rawValue)
        if (
          key.startsWith('iii.tag.') ||
          key === 'iii.session.id' ||
          key === 'iii.session.name' ||
          key === 'iii.message.id'
        ) {
          traceTags[key] = value
        }
        if (attributeProjection.includes(key)) {
          projectedAttributes[key] = value
        }
      }
    }

    const representativeAttributes = spanAttributes(representative)
    const errorCount = ordered.filter(
      (span) => span.status.toLowerCase() === 'error',
    ).length
    const pending = ordered.some(
      (span) => span.pending === true || span.end_time_unix_nano === 0,
    )
    const outcome = traceTags['iii.tag.outcome']

    return [
      {
        trace_id: representative.trace_id,
        name: representative.name,
        start_time_unix_nano: Math.min(
          ...ordered.map((span) => span.start_time_unix_nano),
        ),
        end_time_unix_nano: pending
          ? undefined
          : Math.max(...ordered.map((span) => span.end_time_unix_nano)),
        status:
          errorCount > 0 || outcome === 'failed' || outcome === 'error'
            ? 'error'
            : pending
              ? 'pending'
              : 'ok',
        service_name: representative.service_name || undefined,
        function_id:
          String(
            representativeAttributes['faas.invoked_name'] ??
              representativeAttributes.function_id ??
              '',
          ) || undefined,
        topic:
          String(
            representativeAttributes['messaging.destination.name'] ?? '',
          ) || undefined,
        trace_tags: Object.keys(traceTags).length > 0 ? traceTags : undefined,
        attributes:
          Object.keys(projectedAttributes).length > 0
            ? projectedAttributes
            : undefined,
        span_count: ordered.length,
        error_count: errorCount,
      } satisfies TraceSummary,
    ]
  })
}

/** Normalize the pre-summary `{ spans }` contract only when an older Engine
 * answers the list RPC. New Engines return `{ traces }` unchanged, so the
 * compact transport and server-side totals remain the normal path. */
export function normalizeTracesResponse(
  response: TracesWireResponse,
  options?: TracesFilterParams,
): TracesResponse {
  if ('traces' in response) return response

  const traces = summarizeLegacySpans(
    response.spans,
    options?.attribute_projection,
  )
  return {
    traces,
    // Legacy pagination is span-based. Report the unique rows actually known
    // to this compatibility page instead of presenting a false trace total.
    total: traces.length,
    offset: response.offset,
    limit: response.limit,
  }
}

function asError(err: unknown, fallback: string): Error {
  if (err instanceof Error) return err
  return new Error(errText(err) || fallback)
}

export async function fetchTraces(
  options?: TracesFilterParams,
): Promise<TracesResponse> {
  const offset = options?.offset ?? 0
  const limit = options?.limit ?? 100
  const payload = stripUndefined({ ...options, offset, limit })

  try {
    const client = await getIiiClient()
    const response = await client.trigger<TracesWireResponse>(
      TRACES_RPC_FUNCTIONS.list,
      payload,
    )
    return normalizeTracesResponse(response, options)
  } catch (err) {
    if (isMemoryExporterNotEnabled(err)) {
      return {
        traces: [],
        total: 0,
        offset,
        limit,
        memoryExporterDisabled: true,
      }
    }
    throw asError(err, 'Failed to fetch traces')
  }
}

export async function fetchTraceSpans(
  options?: TracesFilterParams,
): Promise<TraceSpansResponse> {
  const offset = options?.offset ?? 0
  const limit = options?.limit ?? 100
  const payload = stripUndefined({ ...options, offset, limit })

  try {
    const client = await getIiiClient()
    try {
      return await client.trigger<TraceSpansResponse>(
        TRACES_RPC_FUNCTIONS.spans,
        payload,
      )
    } catch (err) {
      if (!isFunctionUnavailable(err)) throw err
      return await client.trigger<TraceSpansResponse>(
        TRACES_RPC_FUNCTIONS.list,
        payload,
      )
    }
  } catch (err) {
    if (isMemoryExporterNotEnabled(err)) {
      return {
        spans: [],
        total: 0,
        offset,
        limit,
        memoryExporterDisabled: true,
      }
    }
    throw asError(err, 'Failed to fetch trace spans')
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
