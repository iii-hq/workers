/**
 * Span transformation utilities for visualization
 * Converts StoredSpan[] to visualization-ready format with computed depth, positioning
 */

import type { SpanTreeNode, StoredSpan } from '../api/traces'

/**
 * Visualization-ready span with computed positioning
 */
export interface VisualizationSpan {
  name: string
  span_id: string
  parent_span_id?: string
  trace_id: string
  duration_ms: number
  status: 'ok' | 'error' | 'unset'
  depth: number
  start_percent: number
  width_percent: number
  attributes: Record<string, unknown>
  events: StoredSpan['events']
  links: StoredSpan['links']
  kind?: string
  service_name?: string
  instrumentation_scope_name?: string
  instrumentation_scope_version?: string
  flags?: number
}

/**
 * Waterfall visualization data
 */
export interface WaterfallData {
  spans: VisualizationSpan[]
  total_duration_ms: number
  span_count: number
}

/**
 * Calculate span depth in the tree.
 *
 * Iterative on purpose: for a deeply nested trace (e.g. an 8000-span
 * long-running workflow where each span is a child of the previous
 * one), a recursive parent walk blows the V8 call stack at ~5-15k
 * frames. The previous recursive `getDepth(span)` would throw
 * `RangeError: Maximum call stack size exceeded` and the React render
 * that called it would crash, leaving the user staring at "loading…".
 */
function calculateDepths(spans: StoredSpan[]): Map<string, number> {
  const depths = new Map<string, number>()
  const spanMap = new Map(spans.map((s) => [s.span_id, s]))

  for (const seed of spans) {
    if (depths.has(seed.span_id)) continue
    // Walk up to a span with a known depth (or a root), pushing each
    // unvisited ancestor. Then resolve depths in reverse on the way
    // back down. Cycle guard via `visiting` prevents infinite loops if
    // the engine ever returns a malformed parent chain.
    const chain: StoredSpan[] = []
    const visiting = new Set<string>()
    let cursor: StoredSpan | undefined = seed
    while (
      cursor !== undefined &&
      !depths.has(cursor.span_id) &&
      !visiting.has(cursor.span_id)
    ) {
      visiting.add(cursor.span_id)
      chain.push(cursor)
      const parentId: string | undefined = cursor.parent_span_id
      cursor = parentId !== undefined ? spanMap.get(parentId) : undefined
    }
    // Base depth: 0 if we hit a root (no parent in the set or a cycle).
    let base =
      cursor !== undefined && depths.has(cursor.span_id)
        ? (depths.get(cursor.span_id) ?? 0)
        : -1
    for (let i = chain.length - 1; i >= 0; i--) {
      base += 1
      depths.set(chain[i].span_id, base)
    }
  }

  return depths
}

/**
 * Threshold for detecting nanosecond timestamps (Jan 1, 2100 in milliseconds)
 */
const NANO_THRESHOLD = 4102444800000

/**
 * Convert timestamp to milliseconds
 * Auto-detects nanosecond vs millisecond timestamps
 */
export function toMs(timestamp: number): number {
  if (!Number.isFinite(timestamp)) return 0
  return timestamp > NANO_THRESHOLD ? timestamp / 1_000_000 : timestamp
}

/**
 * Calculate duration in milliseconds between two timestamps
 * Handles both nanosecond and millisecond timestamps
 */
export function calculateDurationMs(
  startTime: number,
  endTime: number,
): number {
  const startMs = toMs(startTime)
  const endMs = toMs(endTime)
  const duration = endMs - startMs
  return Number.isFinite(duration) && duration >= 0 ? duration : 0
}

/**
 * Normalize a span status into the three UI states.
 *
 * The engine declares `status` as a string, but some OTel encoders emit
 * it as a numeric code (0=unset, 1=ok, 2=error). Accepts `unknown` and
 * coerces defensively so a non-string value (number, null, object)
 * crossing the RPC boundary can never crash a `.toLowerCase()` call.
 */
export function normalizeSpanStatus(status: unknown): 'ok' | 'error' | 'unset' {
  if (status == null) return 'unset'
  const lower = String(status).toLowerCase()
  if (lower === 'error' || lower === '2') return 'error'
  if (lower === 'ok' || lower === '1') return 'ok'
  return 'unset'
}

/**
 * Get span status from status string. Thin wrapper around
 * {@link normalizeSpanStatus} kept for call-site readability.
 */
function getSpanStatus(status: StoredSpan['status']): 'ok' | 'error' | 'unset' {
  return normalizeSpanStatus(status)
}

/**
 * Convert attributes from array-of-tuples to Record.
 * Handles both `[["key","val"], ...]` (engine format) and already-converted Records.
 */
function attributesToRecord(
  attributes: Array<[string, unknown]> | Record<string, unknown> | undefined,
): Record<string, unknown> {
  if (!attributes) return Object.create(null) as Record<string, unknown>

  const record: Record<string, unknown> = Object.create(null)

  if (!Array.isArray(attributes)) {
    for (const [key, value] of Object.entries(attributes)) {
      record[key] = value
    }
    return record
  }

  for (const item of attributes) {
    if (Array.isArray(item) && item.length >= 2) {
      record[String(item[0])] = item[1]
    }
  }
  return record
}

/**
 * Transform raw spans for a specific trace into WaterfallData
 * @param spans - All spans (will be filtered by traceId)
 * @param traceId - Trace ID to filter spans
 * @returns WaterfallData with computed positions and depths
 */
export function toWaterfallData(
  spans: StoredSpan[],
  traceId: string,
): WaterfallData | null {
  const traceSpans = spans.filter((s) => s.trace_id === traceId)

  if (traceSpans.length === 0) {
    return null
  }

  // Calculate trace boundaries (in milliseconds). Reduce-based instead
  // of `Math.min(...arr)` because spread arguments hit a hard cap
  // (~65-125k items in V8) and the spread itself uses stack — both
  // foot-guns for traces with many spans.
  let minStart = Number.POSITIVE_INFINITY
  let maxEnd = Number.NEGATIVE_INFINITY
  for (const s of traceSpans) {
    const start = toMs(s.start_time_unix_nano)
    const end = toMs(s.end_time_unix_nano)
    if (start < minStart) minStart = start
    if (end > maxEnd) maxEnd = end
  }
  const totalDurationMs = maxEnd - minStart

  // Calculate depths
  const depths = calculateDepths(traceSpans)
  const spanMap = new Map(traceSpans.map((s) => [s.span_id, s]))

  // Convert to VisualizationSpan format with percentages
  const visualSpans: VisualizationSpan[] = traceSpans.map((storedSpan) => {
    const durationMs = calculateDurationMs(
      storedSpan.start_time_unix_nano,
      storedSpan.end_time_unix_nano,
    )
    const startOffset = toMs(storedSpan.start_time_unix_nano) - minStart
    const startPercent =
      totalDurationMs > 0 ? (startOffset / totalDurationMs) * 100 : 0
    const widthPercent =
      totalDurationMs > 0 ? (durationMs / totalDurationMs) * 100 : 100

    return {
      name: storedSpan.name,
      span_id: storedSpan.span_id,
      parent_span_id: storedSpan.parent_span_id,
      trace_id: storedSpan.trace_id,
      duration_ms: durationMs,
      status: getSpanStatus(storedSpan.status),
      depth: depths.get(storedSpan.span_id) || 0,
      start_percent: startPercent,
      width_percent: widthPercent,
      attributes: attributesToRecord(storedSpan.attributes),
      events: (storedSpan.events || []).map((e) => ({
        ...e,
        attributes: attributesToRecord(e.attributes),
      })),
      links: storedSpan.links || [],
      kind: storedSpan.kind,
      service_name:
        storedSpan.service_name ||
        (storedSpan.resource?.['service.name'] as string) ||
        undefined,
      instrumentation_scope_name: undefined,
      instrumentation_scope_version: undefined,
      flags: storedSpan.flags,
    }
  })

  // Sort by start time, then by depth
  visualSpans.sort((a, b) => {
    const aStart = toMs(spanMap.get(a.span_id)?.start_time_unix_nano ?? 0)
    const bStart = toMs(spanMap.get(b.span_id)?.start_time_unix_nano ?? 0)
    if (aStart !== bStart) return aStart - bStart
    return a.depth - b.depth
  })

  return {
    spans: visualSpans,
    total_duration_ms: totalDurationMs,
    span_count: visualSpans.length,
  }
}

/**
 * Flatten a SpanTreeNode tree into a flat list of StoredSpan-like objects
 * Depth is computed naturally from tree nesting level
 */
/**
 * Flatten a span tree into a depth-tagged list, in DFS order.
 *
 * Iterative on purpose: an 8000-span trace with a deep parent chain
 * (long-running workflow, recursive crawl, RAG pipeline with nested
 * tool calls) would blow the V8 call stack via the previous
 * `result.push(...flattenTree(children, depth+1))` recursion. The
 * spread itself was a second hazard — `arr.push(...big)` fails with
 * "Maximum call stack size exceeded" once the spread argument count
 * exceeds the engine's argument limit (~65-125k in V8).
 */
function flattenTree(
  nodes: SpanTreeNode[],
): Array<{ span: SpanTreeNode; depth: number }> {
  const result: Array<{ span: SpanTreeNode; depth: number }> = []
  // Use an explicit stack of (node, depth) frames. Push children in
  // reverse so the first child is popped first, preserving DFS order.
  const stack: Array<{ node: SpanTreeNode; depth: number }> = []
  for (let i = nodes.length - 1; i >= 0; i--) {
    stack.push({ node: nodes[i], depth: 0 })
  }
  while (stack.length > 0) {
    const { node, depth } = stack.pop() as { node: SpanTreeNode; depth: number }
    result.push({ span: node, depth })
    const children = node.children
    if (children && children.length > 0) {
      for (let i = children.length - 1; i >= 0; i--) {
        stack.push({ node: children[i], depth: depth + 1 })
      }
    }
  }
  return result
}

/**
 * Transform a trace tree response into WaterfallData
 * Uses the tree structure to compute depth naturally instead of calculating from parent references
 * @param roots - Root span tree nodes from the trace tree API
 * @returns WaterfallData with computed positions and depths
 */
export function treeToWaterfallData(
  roots: SpanTreeNode[],
): WaterfallData | null {
  if (!roots || roots.length === 0) {
    return null
  }

  // Flatten the tree with depth information
  const flatSpans = flattenTree(roots)

  if (flatSpans.length === 0) {
    return null
  }

  // Calculate trace boundaries (in milliseconds). Reduce-based for the
  // same reason as `toWaterfallData` above: avoid the spread argument
  // cap for traces with many spans.
  let minStart = Number.POSITIVE_INFINITY
  let maxEnd = Number.NEGATIVE_INFINITY
  for (const { span } of flatSpans) {
    const start = toMs(span.start_time_unix_nano)
    const end = toMs(span.end_time_unix_nano)
    if (start < minStart) minStart = start
    if (end > maxEnd) maxEnd = end
  }
  const totalDurationMs = maxEnd - minStart

  // Convert to VisualizationSpan format
  const visualSpans: VisualizationSpan[] = flatSpans.map(({ span, depth }) => {
    const durationMs = calculateDurationMs(
      span.start_time_unix_nano,
      span.end_time_unix_nano,
    )
    const startOffset = toMs(span.start_time_unix_nano) - minStart
    const startPercent =
      totalDurationMs > 0 ? (startOffset / totalDurationMs) * 100 : 0
    const widthPercent =
      totalDurationMs > 0 ? (durationMs / totalDurationMs) * 100 : 100

    return {
      name: span.name,
      span_id: span.span_id,
      parent_span_id: span.parent_span_id,
      trace_id: span.trace_id,
      duration_ms: durationMs,
      status: getSpanStatus(span.status),
      depth,
      start_percent: startPercent,
      width_percent: widthPercent,
      attributes: attributesToRecord(span.attributes || []),
      events: (span.events || []).map((e) => ({
        ...e,
        attributes: attributesToRecord(e.attributes),
      })),
      links: span.links || [],
      kind: span.kind,
      service_name:
        span.service_name ||
        (span.resource?.['service.name'] as string) ||
        undefined,
      instrumentation_scope_name: undefined,
      instrumentation_scope_version: undefined,
      flags: span.flags,
    }
  })

  return {
    spans: visualSpans,
    total_duration_ms: totalDurationMs,
    span_count: visualSpans.length,
  }
}
