/**
 * Span-filter selection for the trace detail views: which span groups and
 * which workers are hidden by the funnel menu. ONE selection is shared by
 * the timeline and waterfall views and persisted in the server-side
 * `console` configuration entry under `traces.spanFilters`, so it follows
 * the engine (and every browser tab pointing at it) rather than living
 * per-component.
 *
 * Pure module — parse/serialize only. Transport lives in
 * `@/lib/console-config`, React state in `hooks/useSpanFilterSelection.ts`,
 * the hiding mechanics in `components/timeline/spanVisibility.ts`.
 */

export interface SpanFilterSelection {
  /** Hidden span-group keys (owning function id — see `traceTimelineFilters`). */
  hiddenGroups: ReadonlySet<string>
  /** Hidden worker names (`getWorkerName`). */
  hiddenWorkers: ReadonlySet<string>
}

/** Selection plus the mutations the funnel menu needs. */
export interface SpanFilterControls extends SpanFilterSelection {
  toggleGroup: (key: string) => void
  toggleWorker: (key: string) => void
  clear: () => void
}

export const EMPTY_SPAN_FILTERS: SpanFilterSelection = {
  hiddenGroups: new Set(),
  hiddenWorkers: new Set(),
}

function parseKeySet(value: unknown): ReadonlySet<string> {
  if (!Array.isArray(value)) return new Set()
  return new Set(value.filter((v): v is string => typeof v === 'string'))
}

/** Parse `traces.spanFilters` out of the raw console-config value. */
export function parseSpanFilters(
  configValue: Record<string, unknown>,
): SpanFilterSelection {
  const traces = configValue.traces
  if (!traces || typeof traces !== 'object') return EMPTY_SPAN_FILTERS
  const filters = (traces as Record<string, unknown>).spanFilters
  if (!filters || typeof filters !== 'object') return EMPTY_SPAN_FILTERS
  const f = filters as Record<string, unknown>
  return {
    hiddenGroups: parseKeySet(f.hiddenGroups),
    hiddenWorkers: parseKeySet(f.hiddenWorkers),
  }
}

/** Write the selection back into a (copied) console-config value. */
export function withSpanFilters(
  configValue: Record<string, unknown>,
  selection: SpanFilterSelection,
): Record<string, unknown> {
  const traces =
    configValue.traces && typeof configValue.traces === 'object'
      ? { ...(configValue.traces as Record<string, unknown>) }
      : {}
  traces.spanFilters = {
    hiddenGroups: [...selection.hiddenGroups].sort(),
    hiddenWorkers: [...selection.hiddenWorkers].sort(),
  }
  return { ...configValue, traces }
}
