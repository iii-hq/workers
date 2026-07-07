// Pure mapping from an OTel `StoredSpan` (server shape) to a
// `TraceListItem` (UI view-model used by the flat-list view in the
// TRACES tab). Extracted from `useTraceData` so a porter can run the
// same mapping in non-React contexts (e.g. tests, server-side render,
// a different SDK transport that emits the same span shape).

import type { StoredSpan } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import { toMs } from './traceTransform'

/**
 * Normalize a span's attributes to a flat object.
 *
 * The engine emits attributes in two shapes depending on the encoder:
 * - Array of `[key, value]` tuples (newer protobuf-derived shape)
 * - Plain object (older JSON shape)
 *
 * Returns a fresh object so callers can mutate freely. Non-array,
 * non-object inputs (null, undefined) return `{}` rather than throwing.
 */
export function normalizeSpanAttributes(
  attrs: StoredSpan['attributes'] | Record<string, unknown> | undefined,
): Record<string, unknown> {
  const out: Record<string, unknown> = {}
  if (!attrs) return out
  if (Array.isArray(attrs)) {
    for (const item of attrs) {
      if (Array.isArray(item) && item.length >= 2) {
        out[String(item[0])] = item[1]
      }
    }
    return out
  }
  if (typeof attrs === 'object') {
    Object.assign(out, attrs)
  }
  return out
}

/**
 * Map one stored span into the flat-list TRACES row view-model.
 *
 * Status normalization: any case variant of "error" maps to 'error';
 * everything else (including 'OK', 'unset', 'UNSET') maps to 'ok'.
 * The flat-list view treats 'unset' as 'ok' because it has no neutral
 * indicator (the session-aggregate view has its own).
 *
 * Function-ID and topic come from OTel semantic attributes:
 * - `faas.invoked_name` is the OTel-standard FaaS function attribute;
 *   `function_id` is the iii-engine-specific fallback.
 * - `messaging.destination.name` is the OTel-standard queue/topic
 *   attribute (set when the span is an enqueue).
 *
 * `spanCount` is always 1 because the flat-list view treats each row
 * as one trace; the engine's aggregate counts live in the
 * group-by/`SessionDetailPanel` path.
 */
export function mapSpanToListItem(span: StoredSpan): TraceListItem {
  // A live (pending) span from the engine: still running, end sentinel 0.
  const pending = !!span.pending || span.end_time_unix_nano === 0
  const startTime = toMs(span.start_time_unix_nano)
  const endTime = pending ? undefined : toMs(span.end_time_unix_nano)
  const duration = endTime === undefined ? undefined : endTime - startTime
  const attrs = normalizeSpanAttributes(span.attributes)

  const functionId = (attrs['faas.invoked_name'] || attrs.function_id) as
    | string
    | undefined
  const topic = attrs['messaging.destination.name'] as string | undefined

  return {
    traceId: span.trace_id,
    rootOperation: span.name,
    functionId,
    topic,
    status:
      span.status.toLowerCase() === 'error'
        ? 'error'
        : pending
          ? 'pending'
          : 'ok',
    startTime,
    endTime,
    duration,
    spanCount: 1,
    workers: [span.service_name || 'unknown'],
  }
}

/**
 * Collapse a span list to one representative span per `trace_id` — the root
 * (no parent) when present, else the earliest-started span.
 *
 * The flat-list TRACES view is one row per trace. A plain `engine::traces::list`
 * already returns root spans only, but the SEARCH path passes
 * `search_all_spans: true`, which returns EVERY span of each matching trace
 * (so a query like `harness::send` matches a child span and the engine
 * hands back the whole turn). Collapsing here keeps the list one-row-per-trace
 * regardless, and is a no-op for the non-search response (already roots).
 */
export function dedupeToTraceRoots(
  spans: ReadonlyArray<StoredSpan>,
): StoredSpan[] {
  const byTrace = new Map<string, StoredSpan>()
  for (const span of spans) {
    const existing = byTrace.get(span.trace_id)
    if (!existing) {
      byTrace.set(span.trace_id, span)
      continue
    }
    const spanIsRoot = !span.parent_span_id
    const existingIsRoot = !existing.parent_span_id
    if (spanIsRoot && !existingIsRoot) {
      byTrace.set(span.trace_id, span)
    } else if (
      spanIsRoot === existingIsRoot &&
      span.start_time_unix_nano < existing.start_time_unix_nano
    ) {
      byTrace.set(span.trace_id, span)
    }
  }
  return [...byTrace.values()]
}

/**
 * Stable identity fingerprint for a list of TraceListItems. Used by
 * the hook to dedupe back-to-back fetches that return the same rows.
 *
 * Joins all trace IDs in order. Earlier versions sampled only first +
 * last + count, which would have missed middle-only churn if the sort
 * order ever flipped. At the 500-trace ceiling × 32-char IDs the
 * fingerprint is ~16KB — cheap to compare.
 */
export function fingerprintTraceList(
  traces: ReadonlyArray<TraceListItem>,
): string {
  return `${traces.length}:${traces.map((t) => t.traceId).join(',')}`
}
