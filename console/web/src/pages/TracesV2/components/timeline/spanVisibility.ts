/**
 * Span visibility for the trace detail views (timeline + waterfall): spans
 * are grouped by a caller-supplied key (the page groups by owning function
 * id — see `lib/traceTimelineFilters.ts`), by worker, and by INTERNAL
 * family (`iii.tag.hidden`), and the filter menu lists the sections
 * most-populated first.
 *
 * Hiding removes ONLY the matched span itself — its children stay visible
 * and re-attach to the hidden span's parent (the nearest visible ancestor),
 * so the hierarchy stays connected: hiding the `harness::turn` dispatch
 * wrappers leaves the turn's step span parented under `harness::send`;
 * hiding `context::assemble` leaves its `router::models::get` child in
 * place. Work caused by a hidden span is still real work — a hide is a
 * de-noise, not a subtree collapse. Chains of plumbing hide together
 * anyway when each span matches on its own (baggage smears `iii.tag.hidden`
 * across a tagged delivery subtree, so every span of it matches).
 *
 * This module owns only the mechanics; what a span GROUP is lives with the
 * caller, while workers are fixed to `getWorkerName` and internal families
 * to `internalFamilyOf`.
 */

import type { SpanFilterSelection } from '../../lib/spanFilters'
import { internalFamilyOf } from '../../lib/spanLabel'
import type { VisualizationSpan, WaterfallData } from '../../lib/traceTransform'
import { getWorkerName } from '../../lib/traceUtils'

/**
 * Grouping key for a span, or null when the span belongs to no group.
 * Callers that group across a whole trace get the trace's spans by id as a
 * second argument (parent lookups, e.g. tag-root detection).
 */
export type SpanGroupKey = (
  span: VisualizationSpan,
  spansById?: ReadonlyMap<string, VisualizationSpan>,
) => string | null

/** Grouping key for the filter menu's workers section. */
export const workerGroupKey: SpanGroupKey = (span) => getWorkerName(span)

/** The resolved filter-key triple a bar carries (a `TimelineSpan`'s
 *  `groupKey` / `workerKey` / `internalKey`). */
export interface SpanFilterKeys {
  groupKey?: string
  workerKey?: string
  internalKey?: string
}

/**
 * THE hidden-span predicate: whether the selection hides a span carrying
 * these keys. Internal spans hide by DEFAULT — `shownInternal` lists the
 * families the user revealed; groups and workers hide only when picked.
 * Every surface that applies the funnel selection (the strip's bars, the
 * detail views via [`applyHiddenSpanFilters`], the trace list's rows via
 * `hooks/useSpanFilteredTraceRows`) routes through this one function so
 * they can never disagree on what "hidden" means.
 */
export function isSpanBarHidden(
  keys: SpanFilterKeys,
  selection: SpanFilterSelection,
): boolean {
  if (
    keys.internalKey != null &&
    !selection.shownInternal.has(keys.internalKey)
  ) {
    return true
  }
  if (keys.groupKey != null && selection.hiddenGroups.has(keys.groupKey)) {
    return true
  }
  return keys.workerKey != null && selection.hiddenWorkers.has(keys.workerKey)
}

export interface SpanGroup {
  key: string
  /** Spans carrying this key (subtree descendants not included). */
  count: number
}

/**
 * Group spans by `keyOf`, most-populated groups first (ties break
 * alphabetically) — the busiest call families float to the top of the
 * filter menu. Generic over the span shape: the detail views group
 * `VisualizationSpan`s, the masthead strip groups its `TimelineSpan` bars.
 */
export function deriveSpanGroups<T>(
  spans: readonly T[],
  keyOf: (span: T) => string | null | undefined,
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

/** Guard against malformed parent chains; real traces are far shallower. */
const MAX_PARENT_WALK = 1000

/**
 * Apply the filter selection (hidden span groups + hidden workers +
 * default-hidden internal families) to the waterfall.
 *
 * Only the matched spans disappear. Each surviving span whose parent was
 * hidden re-parents to its nearest surviving ancestor, and `depth` is
 * recomputed from the rewritten chains so indentation stays consistent. A
 * parent id that was never in the trace is kept as-is (external/distributed
 * parents already render as local roots). The time window
 * (`total_duration_ms`) is deliberately preserved — filtering noise out
 * must not rescale the remaining bars. Returns `data` unchanged when
 * nothing matches.
 */
export function applyHiddenSpanFilters(
  data: WaterfallData,
  keyOf: SpanGroupKey,
  selection: SpanFilterSelection,
): WaterfallData {
  const spansById = new Map(data.spans.map((s) => [s.span_id, s]))
  const matches = (span: VisualizationSpan) =>
    isSpanBarHidden(
      {
        groupKey: keyOf(span, spansById) ?? undefined,
        workerKey: getWorkerName(span),
        internalKey: internalFamilyOf(span.attributes) ?? undefined,
      },
      selection,
    )

  const hidden = new Set<string>()
  for (const span of data.spans) {
    if (matches(span)) hidden.add(span.span_id)
  }
  if (hidden.size === 0) return data

  // Nearest surviving ancestor: hop over hidden parents; stop at the first
  // visible one, at a parent that isn't in the trace (kept verbatim — the
  // tree builders already treat unknown parents as roots), or at the top.
  const visibleParentId = (span: VisualizationSpan): string | undefined => {
    let parentId = span.parent_span_id
    const seen = new Set<string>([span.span_id])
    for (let hops = 0; parentId && hops < MAX_PARENT_WALK; hops++) {
      if (!hidden.has(parentId) || seen.has(parentId)) break
      seen.add(parentId)
      parentId = spansById.get(parentId)?.parent_span_id
    }
    // A malformed cycle can walk back to the span itself — that must not
    // become a self-parent (the tree builders would loop).
    return parentId === span.span_id ? undefined : (parentId ?? undefined)
  }

  const spans = data.spans
    .filter((s) => !hidden.has(s.span_id))
    .map((s) => {
      const parent = visibleParentId(s)
      return parent === (s.parent_span_id ?? undefined)
        ? s
        : { ...s, parent_span_id: parent }
    })

  // Depth from the rewritten chains, so a promoted span indents one level
  // under its new parent instead of keeping its old, deeper offset.
  const byId = new Map(spans.map((s) => [s.span_id, s]))
  const depths = new Map<string, number>()
  const depthOf = (span: VisualizationSpan): number => {
    // Iterative: collect the unresolved ancestor chain, then assign.
    const chain: VisualizationSpan[] = []
    let current: VisualizationSpan | undefined = span
    while (
      current &&
      !depths.has(current.span_id) &&
      chain.length < MAX_PARENT_WALK
    ) {
      chain.push(current)
      current = current.parent_span_id
        ? byId.get(current.parent_span_id)
        : undefined
      // A cycle would revisit a chain member; treat it as a root boundary.
      if (current && chain.some((s) => s.span_id === current?.span_id)) {
        current = undefined
      }
    }
    let depth = current ? (depths.get(current.span_id) ?? 0) : -1
    for (let i = chain.length - 1; i >= 0; i--) {
      depth += 1
      depths.set(chain[i].span_id, depth)
    }
    return depths.get(span.span_id) ?? 0
  }
  const withDepths = spans.map((s) => {
    const depth = depthOf(s)
    return depth === s.depth ? s : { ...s, depth }
  })

  return { ...data, spans: withDepths, span_count: withDepths.length }
}

/**
 * The masthead strips' equivalent of the promotion rule: keep only the bars
 * `keep` accepts, re-pointing each kept bar's `parentId` through hidden
 * bars to its nearest KEPT ancestor — a hidden bar's children stay in the
 * hierarchy as children of its parent. A `parentId` that isn't in the feed
 * at all is preserved (the layouts already treat unresolvable parents as
 * roots, and the parent may simply not have arrived yet).
 */
export function reparentThroughHidden<
  T extends { id: string; parentId?: string },
>(all: readonly T[], keep: (bar: T) => boolean): T[] {
  const byId = new Map(all.map((b) => [b.id, b]))
  const kept: T[] = []
  for (const bar of all) {
    if (!keep(bar)) continue
    let parentId = bar.parentId
    const seen = new Set<string>([bar.id])
    for (let hops = 0; parentId && hops < MAX_PARENT_WALK; hops++) {
      const parent = byId.get(parentId)
      if (!parent || keep(parent) || seen.has(parentId)) break
      seen.add(parentId)
      parentId = parent.parentId
    }
    // A malformed cycle can walk back to the bar itself — never emit a
    // self-parented bar.
    if (parentId === bar.id) parentId = undefined
    kept.push(parentId === bar.parentId ? bar : { ...bar, parentId })
  }
  return kept
}
