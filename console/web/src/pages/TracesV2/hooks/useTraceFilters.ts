import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { TracesFilterParams } from '../api/traces'
import type { GroupByOption } from '../lib/groupTraces'
import { buildFilterParams, countActiveFilters } from '../lib/traceFilters'

export interface TraceFilterState {
  workerName?: string
  operationName?: string
  status?: 'ok' | 'error' | 'unset' | null
  minDurationMs?: number | null
  maxDurationMs?: number | null
  startTime?: number | null
  endTime?: number | null
  attributes?: [string, string][]
  sortBy?: 'start_time' | 'duration' | 'service_name'
  sortOrder?: 'asc' | 'desc'
  /**
   * Group spans by an attribute value. When set to anything other than
   * 'none', the list view renders collapsible group rows fed by
   * `engine::traces::group_by`. Default 'none'.
   */
  groupBy?: GroupByOption
  /**
   * Root functions hidden from the list (exact `function_id` match).
   * Applied client-side over the seed + streamed rows, so toggling never
   * refetches and the live-append path stays intact.
   */
  hiddenFunctions?: string[]
  /** How each list row is labelled. Default 'function'. */
  labelMode?: 'function' | 'span-name' | 'attribute'
  /** Attribute key for labelMode 'attribute' (e.g. `iii.tag.message`). */
  labelAttribute?: string
  page: number
  pageSize: number
}

const defaultFilters: TraceFilterState = {
  workerName: undefined,
  operationName: undefined,
  status: null,
  minDurationMs: null,
  maxDurationMs: null,
  startTime: null,
  endTime: null,
  attributes: undefined,
  sortBy: 'start_time',
  sortOrder: 'desc',
  groupBy: 'none',
  hiddenFunctions: undefined,
  labelMode: 'function',
  labelAttribute: undefined,
  page: 1,
  pageSize: 50,
}

/**
 * Hook for managing trace filter state with server-side pagination.
 *
 * Features:
 * - 300ms debouncing for text inputs (workerName, operationName)
 * - Automatic page reset when filters change
 * - Type-safe API parameter conversion (camelCase → snake_case)
 * - Range validation (auto-swaps min/max if invalid)
 *
 * @returns {Object} Filter state and control functions
 * @returns {TraceFilterState} filters - Current filter state
 * @returns {Function} updateFilter - Update a single filter field
 * @returns {Function} resetFilters - Reset all filters to defaults
 * @returns {Function} getActiveFilterCount - Count non-default filters
 * @returns {Function} getApiParams - Convert to API-ready parameters
 *
 * @example
 * const { filters, updateFilter, getApiParams } = useTraceFilters()
 *
 * updateFilter('workerName', 'api-gateway')
 * updateFilter('minDurationMs', 100)
 *
 * const apiParams = getApiParams()
 * // { service_name: "api-gateway", min_duration_ms: 100, offset: 0, limit: 50 }
 */
export function useTraceFilters() {
  const [filters, setFilters] = useState<TraceFilterState>(defaultFilters)
  const [debouncedWorkerName, setDebouncedWorkerName] = useState<
    string | undefined
  >(undefined)
  const [debouncedOperationName, setDebouncedOperationName] = useState<
    string | undefined
  >(undefined)

  const workerNameTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const operationNameTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  )

  const [validationWarnings, setValidationWarnings] = useState<{
    durationSwapped?: boolean
    timeRangeSwapped?: boolean
  }>({})

  useEffect(() => {
    return () => {
      if (workerNameTimerRef.current) clearTimeout(workerNameTimerRef.current)
      if (operationNameTimerRef.current)
        clearTimeout(operationNameTimerRef.current)
    }
  }, [])

  /** Update a single filter field. Resets page to 1 when any filter changes (except page/pageSize). */
  const updateFilter = useCallback(
    (key: keyof TraceFilterState, value: unknown) => {
      setFilters((prev) => {
        const newFilters = { ...prev, [key]: value }

        // Reset page to 1 when any filter changes (except page/pageSize)
        if (key !== 'page' && key !== 'pageSize') {
          newFilters.page = 1
        }

        return newFilters
      })

      // Handle debouncing for text inputs
      if (key === 'workerName') {
        if (workerNameTimerRef.current) clearTimeout(workerNameTimerRef.current)
        const timer = setTimeout(() => {
          setDebouncedWorkerName(value as string | undefined)
        }, 300)
        workerNameTimerRef.current = timer
      }

      if (key === 'operationName') {
        if (operationNameTimerRef.current)
          clearTimeout(operationNameTimerRef.current)
        const timer = setTimeout(() => {
          setDebouncedOperationName(value as string | undefined)
        }, 300)
        operationNameTimerRef.current = timer
      }
    },
    [],
  )

  const resetFilters = useCallback(() => {
    setFilters(defaultFilters)
    setDebouncedWorkerName(undefined)
    setDebouncedOperationName(undefined)
    if (workerNameTimerRef.current) clearTimeout(workerNameTimerRef.current)
    if (operationNameTimerRef.current)
      clearTimeout(operationNameTimerRef.current)
  }, [])

  /**
   * Replace the whole filter state in one shot (view switching): defaults +
   * the view's snapshot, keeping the current pageSize. Debounced mirrors
   * sync immediately so the list doesn't lag the switch by 300ms.
   */
  const replaceFilters = useCallback((next: Partial<TraceFilterState>) => {
    setFilters((prev) => ({
      ...defaultFilters,
      pageSize: prev.pageSize,
      ...next,
    }))
    setDebouncedWorkerName(next.workerName)
    setDebouncedOperationName(next.operationName)
    if (workerNameTimerRef.current) clearTimeout(workerNameTimerRef.current)
    if (operationNameTimerRef.current)
      clearTimeout(operationNameTimerRef.current)
  }, [])

  const getActiveFilterCount = useCallback(
    () => countActiveFilters(filters),
    [filters],
  )

  // Filter-only params (excludes pagination). Recomputes on any
  // `filters` identity change — including page/pageSize — but the
  // result identity downstream isn't load-bearing: TanStack Query
  // compares queryKeys structurally, so identical content (e.g. when
  // only page changed) doesn't trigger a refetch. Delegation to
  // `buildFilterParams` keeps the swap-validation logic pure and
  // testable (see lib/traceFilters.test.ts).
  const { filterOnlyParams, computedWarnings } = useMemo(() => {
    const { params, warnings } = buildFilterParams(filters)
    return { filterOnlyParams: params, computedWarnings: warnings }
  }, [filters])

  // Full params including pagination offset/limit
  const apiParams = useMemo(
    () => ({
      ...filterOnlyParams,
      offset: (filters.page - 1) * filters.pageSize,
      limit: filters.pageSize,
    }),
    [filterOnlyParams, filters.page, filters.pageSize],
  )

  useEffect(() => {
    setValidationWarnings(computedWarnings)
  }, [computedWarnings])

  const getApiParams = useCallback((): TracesFilterParams => {
    return apiParams
  }, [apiParams])

  // Returns filter params without pagination - stable when only page changes
  const getFilterOnlyParams = useCallback((): TracesFilterParams => {
    return filterOnlyParams
  }, [filterOnlyParams])

  const clearValidationWarnings = useCallback(() => {
    // Bail when state is already empty. Without this guard the
    // auto-dismiss effect in `TraceFilters` (which calls this whenever
    // no warning flag is true) loops forever: passing a fresh `{}`
    // ref to React's setState forces a re-render, the effect's
    // `validationWarnings` dep changes ref, the effect re-fires, and
    // the cycle repeats. React surfaces this as "Maximum update
    // depth exceeded" and the ErrorBoundary swaps in the
    // "something broke" fallback. By returning the previous reference
    // when nothing changed, React's setState short-circuits the
    // update and the loop can't form.
    setValidationWarnings((prev) => {
      if (!prev.durationSwapped && !prev.timeRangeSwapped) return prev
      return {}
    })
  }, [])

  return {
    filters,
    debouncedWorkerName,
    debouncedOperationName,
    updateFilter,
    resetFilters,
    replaceFilters,
    getActiveFilterCount,
    getApiParams,
    getFilterOnlyParams,
    validationWarnings,
    clearValidationWarnings,
  }
}
