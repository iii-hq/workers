// Pure filter-param building for the TRACES tab.
//
// Extracted from `useTraceFilters` so the range-swap validation logic
// (auto-fix min > max and startTime > endTime) is testable without a
// React render. The hook calls this and surfaces the warnings via a
// state effect.

import type { TracesFilterParams } from '../api/traces'
import type { TraceFilterState } from '../hooks/useTraceFilters'

export interface FilterValidationWarnings {
  durationSwapped?: boolean
  timeRangeSwapped?: boolean
}

export interface BuiltFilterParams {
  params: TracesFilterParams
  warnings: FilterValidationWarnings
}

/**
 * Convert the UI's `TraceFilterState` into the engine's
 * `TracesFilterParams` shape (camelCase → snake_case).
 *
 * Range validation:
 * - When both `minDurationMs` and `maxDurationMs` are set and min > max,
 *   the values are swapped and `warnings.durationSwapped = true`.
 * - When both `startTime` and `endTime` are set and start > end, the
 *   values are swapped and `warnings.timeRangeSwapped = true`.
 * - Single-bounded ranges (only min OR only max set) pass through
 *   unchanged without warnings.
 *
 * Empty / null / undefined fields are not emitted on the output —
 * downstream the engine treats missing fields as "no filter".
 *
 * Pagination is NOT included here; the hook adds offset/limit on top.
 */
export function buildFilterParams(
  filters: TraceFilterState,
): BuiltFilterParams {
  const params: TracesFilterParams = {}
  const warnings: FilterValidationWarnings = {}

  if (filters.serviceName) params.service_name = filters.serviceName
  if (filters.operationName) {
    params.name = filters.operationName
    params.search_all_spans = true
  }
  if (filters.status != null) params.status = filters.status

  if (filters.minDurationMs != null && filters.maxDurationMs != null) {
    if (filters.minDurationMs > filters.maxDurationMs) {
      params.min_duration_ms = filters.maxDurationMs
      params.max_duration_ms = filters.minDurationMs
      warnings.durationSwapped = true
    } else {
      params.min_duration_ms = filters.minDurationMs
      params.max_duration_ms = filters.maxDurationMs
    }
  } else {
    if (filters.minDurationMs != null)
      params.min_duration_ms = filters.minDurationMs
    if (filters.maxDurationMs != null)
      params.max_duration_ms = filters.maxDurationMs
  }

  if (filters.startTime != null && filters.endTime != null) {
    if (filters.startTime > filters.endTime) {
      params.start_time = filters.endTime
      params.end_time = filters.startTime
      warnings.timeRangeSwapped = true
    } else {
      params.start_time = filters.startTime
      params.end_time = filters.endTime
    }
  } else {
    if (filters.startTime != null) params.start_time = filters.startTime
    if (filters.endTime != null) params.end_time = filters.endTime
  }

  if (filters.attributes && filters.attributes.length > 0) {
    params.attributes = filters.attributes
  }
  if (filters.sortBy) params.sort_by = filters.sortBy
  if (filters.sortOrder) params.sort_order = filters.sortOrder

  return { params, warnings }
}

/**
 * Count of "non-default" filters active on a state. Used by the UI
 * to render a badge next to the filter button.
 *
 * Defaults considered:
 * - sortBy === 'start_time' (no count)
 * - sortOrder === 'desc' (no count)
 * - status === null (no count)
 * - other fields: count if present / non-empty
 */
export function countActiveFilters(filters: TraceFilterState): number {
  let count = 0
  if (filters.serviceName) count++
  if (filters.operationName) count++
  if (filters.status != null) count++
  if (filters.minDurationMs !== null) count++
  if (filters.maxDurationMs !== null) count++
  if (filters.startTime !== null) count++
  if (filters.endTime !== null) count++
  if (filters.attributes && filters.attributes.length > 0) count++
  if (filters.sortBy !== 'start_time') count++
  if (filters.sortOrder !== 'desc') count++
  return count
}
