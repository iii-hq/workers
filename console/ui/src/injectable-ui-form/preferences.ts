import type { JsonValue } from '@iii-dev/console-ui'

export type JsonObject = { [key: string]: JsonValue }

export interface TraceViewSummary {
  id: string
  name: string
  value: JsonObject
}

export const TRACE_FILTER_KEYS = ['hiddenGroups', 'hiddenWorkers', 'shownGroups', 'shownInternal'] as const

export type TraceFilterKey = (typeof TRACE_FILTER_KEYS)[number]

export function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

export function stringList(value: JsonValue | undefined): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
}

export function traceViews(value: JsonValue): TraceViewSummary[] {
  const views = asObject(asObject(value).traces).views
  if (!Array.isArray(views)) return []
  return views.flatMap((entry) => {
    const view = asObject(entry)
    const id = typeof view.id === 'string' ? view.id : ''
    const name = typeof view.name === 'string' ? view.name : ''
    return id && name ? [{ id, name, value: view }] : []
  })
}

export function activeTraceViewId(value: JsonValue): string | null | undefined {
  const traces = asObject(asObject(value).traces)
  if (!('activeViewId' in traces)) return undefined
  return typeof traces.activeViewId === 'string' && traces.activeViewId ? traces.activeViewId : null
}

function updateTraces(value: JsonValue, update: (traces: JsonObject) => JsonObject): JsonObject {
  const root = asObject(value)
  return { ...root, traces: update(asObject(root.traces)) }
}

export function withFollowTurns(value: JsonValue, enabled: boolean): JsonObject {
  return updateTraces(value, (traces) => ({ ...traces, followTurns: enabled }))
}

export function withActiveTraceView(value: JsonValue, id: string | null): JsonObject {
  return updateTraces(value, (traces) => ({ ...traces, activeViewId: id }))
}

export function renameTraceView(value: JsonValue, id: string, name: string): JsonObject {
  return updateTraces(value, (traces) => ({
    ...traces,
    views: Array.isArray(traces.views)
      ? traces.views.map((entry) => {
          const view = asObject(entry)
          return view.id === id ? { ...view, name } : entry
        })
      : [],
  }))
}

export function removeTraceView(value: JsonValue, id: string): JsonObject {
  return updateTraces(value, (traces) => {
    const views = Array.isArray(traces.views) ? traces.views.filter((entry) => asObject(entry).id !== id) : []
    return {
      ...traces,
      views,
      ...(traces.activeViewId === id ? { activeViewId: null } : {}),
    }
  })
}

export function addTraceView(value: JsonValue, id: string, name = 'New view'): JsonObject {
  return updateTraces(value, (traces) => ({
    ...traces,
    views: [
      ...(Array.isArray(traces.views) ? traces.views : []),
      {
        id,
        name,
        groupBy: 'none',
        hiddenFunctions: [],
        label: { mode: 'function' },
        filters: {},
      },
    ],
    activeViewId: id,
  }))
}

export function traceFilterList(value: JsonValue, key: TraceFilterKey): string[] {
  const filters = asObject(asObject(asObject(value).traces).spanFilters)
  return stringList(filters[key])
}

export function withTraceFilterList(value: JsonValue, key: TraceFilterKey, entries: readonly string[]): JsonObject {
  return updateTraces(value, (traces) => ({
    ...traces,
    spanFilters: {
      ...asObject(traces.spanFilters),
      [key]: [...new Set(entries.map((entry) => entry.trim()).filter(Boolean))],
    },
  }))
}

export function newTraceViewId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return `view-${crypto.randomUUID()}`
  }
  return `view-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}
