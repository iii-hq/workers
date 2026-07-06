/**
 * Span-group visibility for the trace detail timeline: spans are grouped by
 * a caller-supplied key (the page groups by owning function id — see
 * `lib/traceTimelineFilters.ts`), the filter menu lists the groups
 * most-populated first, and hiding a group hides each of its spans TOGETHER
 * WITH its descendants — hiding an invocation hides the work it caused,
 * never orphaning children. This module owns only the mechanics; what a
 * group IS lives with the caller.
 */

import type { VisualizationSpan, WaterfallData } from '../../lib/traceTransform'

/** Grouping key for a span, or null when the span belongs to no group. */
export type SpanGroupKey = (span: VisualizationSpan) => string | null

export interface SpanGroup {
  key: string
  /** Spans carrying this key (subtree descendants not included). */
  count: number
}

/**
 * Group the trace's spans by `keyOf`, most-populated groups first (ties
 * break alphabetically) — the busiest call families float to the top of
 * the filter menu.
 */
export function deriveSpanGroups(
  spans: readonly VisualizationSpan[],
  keyOf: SpanGroupKey,
): SpanGroup[] {
  const counts = new Map<string, number>()
  for (const span of spans) {
    const key = keyOf(span)
    if (!key) continue
    counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  return [...counts]
    .map(([key, count]) => ({ key, count }))
    .sort((a, b) => b.count - a.count || (a.key < b.key ? -1 : 1))
}

/** span_ids of every span matched by `matches`, descendants included. */
function collectSubtreeIds(
  spans: readonly VisualizationSpan[],
  matches: (span: VisualizationSpan) => boolean,
): Set<string> {
  const childrenOf = new Map<string, VisualizationSpan[]>()
  for (const s of spans) {
    if (!s.parent_span_id) continue
    const siblings = childrenOf.get(s.parent_span_id)
    if (siblings) siblings.push(s)
    else childrenOf.set(s.parent_span_id, [s])
  }

  const hidden = new Set<string>()
  const stack = spans.filter(matches)
  while (stack.length > 0) {
    const span = stack.pop() as VisualizationSpan
    if (hidden.has(span.span_id)) continue
    hidden.add(span.span_id)
    const kids = childrenOf.get(span.span_id)
    if (kids) {
      for (const kid of kids) stack.push(kid)
    }
  }
  return hidden
}

/**
 * Apply the hidden groups to the waterfall. The time window
 * (`total_duration_ms`) is deliberately preserved — filtering noise out
 * must not rescale the remaining bars. Returns `data` unchanged when
 * nothing is hidden or nothing matches.
 */
export function applyHiddenSpanGroups(
  data: WaterfallData,
  keyOf: SpanGroupKey,
  hiddenKeys: ReadonlySet<string>,
): WaterfallData {
  if (hiddenKeys.size === 0) return data
  const hidden = collectSubtreeIds(data.spans, (span) => {
    const key = keyOf(span)
    return key != null && hiddenKeys.has(key)
  })
  if (hidden.size === 0) return data

  const spans = data.spans.filter((s) => !hidden.has(s.span_id))
  return { ...data, spans, span_count: spans.length }
}
