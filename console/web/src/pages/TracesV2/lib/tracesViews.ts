// Saved-view model for the TRACES tab.
//
// A view is a named snapshot of EVERYTHING that shapes the list: grouping,
// hidden functions, row-label display, filters, and sort. Views live in the
// server-side `console` configuration entry (`traces.views`), so they follow
// the engine, not the browser; the ACTIVE view id is per-browser
// (localStorage) so two tabs can look at different views.
//
// Pure module — capture/apply/compare only. Transport lives in
// `@/lib/console-config`, React state in `hooks/useTraceViews.ts`.

import type { TraceFilterState } from '../hooks/useTraceFilters'

export type RowLabelMode = 'function' | 'span-name' | 'attribute'

export interface RowLabelConfig {
  mode: RowLabelMode
  /** Attribute key resolved against the row's trace tags, then its own
   *  attributes (only for mode 'attribute'), e.g. `iii.tag.message`. */
  attribute?: string
}

export interface TracesViewConfig {
  /** Attribute key to group by, or 'none'. */
  groupBy: string
  /** Root functions hidden from the list (exact `function_id` match). */
  hiddenFunctions: string[]
  label: RowLabelConfig
  filters: {
    workerName?: string
    operationName?: string
    status?: 'ok' | 'error' | 'unset' | null
    minDurationMs?: number | null
    maxDurationMs?: number | null
    /** Relative time window (e.g. "last 1h" = 3_600_000). Stored as a
     *  duration so the view stays meaningful across days; resolved to
     *  absolute start/end when applied. */
    timeRangeMs?: number | null
    attributes?: [string, string][]
    sortBy?: 'start_time' | 'duration' | 'service_name'
    sortOrder?: 'asc' | 'desc'
  }
}

export interface TracesView extends TracesViewConfig {
  id: string
  name: string
}

/** Snapshot the live filter state into a view config. */
export function captureViewConfig(filters: TraceFilterState): TracesViewConfig {
  const timeRangeMs =
    filters.startTime != null && filters.endTime != null
      ? filters.endTime - filters.startTime
      : null
  return normalizeViewConfig({
    groupBy: filters.groupBy ?? 'none',
    hiddenFunctions: filters.hiddenFunctions ?? [],
    label: {
      mode: filters.labelMode ?? 'function',
      attribute: filters.labelAttribute,
    },
    filters: {
      workerName: filters.workerName,
      operationName: filters.operationName,
      status: filters.status ?? null,
      minDurationMs: filters.minDurationMs ?? null,
      maxDurationMs: filters.maxDurationMs ?? null,
      timeRangeMs,
      attributes: filters.attributes,
      sortBy: filters.sortBy,
      sortOrder: filters.sortOrder,
    },
  })
}

/**
 * Expand a view config back into filter-state fields (the caller spreads
 * this over the defaults). Relative time windows resolve against "now".
 */
export function applyViewConfig(
  config: TracesViewConfig,
): Partial<TraceFilterState> {
  const now = Date.now()
  const c = normalizeViewConfig(config)
  return {
    groupBy: c.groupBy,
    hiddenFunctions: c.hiddenFunctions,
    labelMode: c.label.mode,
    labelAttribute: c.label.attribute,
    workerName: c.filters.workerName,
    operationName: c.filters.operationName,
    status: c.filters.status ?? null,
    minDurationMs: c.filters.minDurationMs ?? null,
    maxDurationMs: c.filters.maxDurationMs ?? null,
    startTime: c.filters.timeRangeMs ? now - c.filters.timeRangeMs : null,
    endTime: c.filters.timeRangeMs ? now : null,
    attributes: c.filters.attributes,
    sortBy: c.filters.sortBy ?? 'start_time',
    sortOrder: c.filters.sortOrder ?? 'desc',
    page: 1,
  }
}

/** Canonical form so equality checks ignore undefined-vs-missing noise. */
export function normalizeViewConfig(
  config: TracesViewConfig,
): TracesViewConfig {
  const f = config.filters ?? {}
  // Views saved before the service→worker rename persisted this filter as
  // `serviceName` (server-side config, so it outlives the frontend build).
  const workerName = f.workerName ?? (f as { serviceName?: string }).serviceName
  return {
    groupBy: config.groupBy || 'none',
    hiddenFunctions: [...(config.hiddenFunctions ?? [])].sort(),
    label: {
      mode: config.label?.mode ?? 'function',
      ...(config.label?.mode === 'attribute' && config.label.attribute
        ? { attribute: config.label.attribute }
        : {}),
    },
    filters: {
      ...(workerName ? { workerName } : {}),
      ...(f.operationName ? { operationName: f.operationName } : {}),
      ...(f.status ? { status: f.status } : {}),
      ...(f.minDurationMs != null ? { minDurationMs: f.minDurationMs } : {}),
      ...(f.maxDurationMs != null ? { maxDurationMs: f.maxDurationMs } : {}),
      ...(f.timeRangeMs != null ? { timeRangeMs: f.timeRangeMs } : {}),
      ...(f.attributes && f.attributes.length > 0
        ? { attributes: f.attributes }
        : {}),
      ...(f.sortBy && f.sortBy !== 'start_time' ? { sortBy: f.sortBy } : {}),
      ...(f.sortOrder && f.sortOrder !== 'desc'
        ? { sortOrder: f.sortOrder }
        : {}),
    },
  }
}

/** True when the live state diverges from the saved view ("modified" dot). */
export function viewConfigEquals(
  a: TracesViewConfig,
  b: TracesViewConfig,
): boolean {
  return (
    JSON.stringify(normalizeViewConfig(a)) ===
    JSON.stringify(normalizeViewConfig(b))
  )
}

/** Parse the `traces.views` array out of the raw console-config value. */
export function parseViews(configValue: Record<string, unknown>): TracesView[] {
  const traces = configValue.traces
  if (!traces || typeof traces !== 'object') return []
  const views = (traces as Record<string, unknown>).views
  if (!Array.isArray(views)) return []
  return views.filter(
    (v): v is TracesView =>
      !!v &&
      typeof v === 'object' &&
      typeof (v as TracesView).id === 'string' &&
      typeof (v as TracesView).name === 'string',
  )
}

/** Write the views array back into a (copied) console-config value. */
export function withViews(
  configValue: Record<string, unknown>,
  views: TracesView[],
): Record<string, unknown> {
  const traces =
    configValue.traces && typeof configValue.traces === 'object'
      ? { ...(configValue.traces as Record<string, unknown>) }
      : {}
  traces.views = views
  return { ...configValue, traces }
}

export function newViewId(): string {
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `view-${crypto.randomUUID()}`
  }
  return `view-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}
