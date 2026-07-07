// Tests for the pure filter-param building extracted from
// `useTraceFilters`. Covers happy-path translation (camelCase ->
// snake_case), single-bounded ranges, and the swap-validation paths
// that auto-correct inverted ranges with a warning flag.

import { describe, expect, it } from 'vitest'
import type { TraceFilterState } from '../hooks/useTraceFilters'
import { buildFilterParams, countActiveFilters } from './traceFilters'

function makeFilters(
  overrides: Partial<TraceFilterState> = {},
): TraceFilterState {
  return {
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
    page: 1,
    pageSize: 50,
    ...overrides,
  }
}

describe('buildFilterParams — happy path', () => {
  it('emits no fields when filters are at defaults', () => {
    const { params, warnings } = buildFilterParams(makeFilters())
    // sortBy + sortOrder always pass through (server uses them); other
    // fields are absent.
    expect(params).toEqual({ sort_by: 'start_time', sort_order: 'desc' })
    expect(warnings).toEqual({})
  })

  it('translates workerName -> service_name', () => {
    const { params } = buildFilterParams(
      makeFilters({ workerName: 'api-gateway' }),
    )
    expect(params.service_name).toBe('api-gateway')
  })

  it('translates operationName -> name + search_all_spans flag', () => {
    const { params } = buildFilterParams(
      makeFilters({ operationName: 'POST /api/users' }),
    )
    expect(params.name).toBe('POST /api/users')
    expect(params.search_all_spans).toBe(true)
  })

  it('translates status when non-null', () => {
    expect(
      buildFilterParams(makeFilters({ status: 'error' })).params.status,
    ).toBe('error')
    expect(
      buildFilterParams(makeFilters({ status: null })).params.status,
    ).toBeUndefined()
  })

  it('forwards attributes when non-empty', () => {
    const attrs: [string, string][] = [
      ['http.method', 'GET'],
      ['http.status_code', '500'],
    ]
    expect(
      buildFilterParams(makeFilters({ attributes: attrs })).params.attributes,
    ).toEqual(attrs)
  })

  it('drops attributes when array is empty', () => {
    expect(
      buildFilterParams(makeFilters({ attributes: [] })).params.attributes,
    ).toBeUndefined()
  })
})

describe('buildFilterParams — duration range', () => {
  it('passes min and max through unchanged when min <= max', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ minDurationMs: 100, maxDurationMs: 500 }),
    )
    expect(params.min_duration_ms).toBe(100)
    expect(params.max_duration_ms).toBe(500)
    expect(warnings.durationSwapped).toBeUndefined()
  })

  it('swaps min/max and flags warning when inverted', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ minDurationMs: 500, maxDurationMs: 100 }),
    )
    expect(params.min_duration_ms).toBe(100)
    expect(params.max_duration_ms).toBe(500)
    expect(warnings.durationSwapped).toBe(true)
  })

  it('handles single-bounded min only', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ minDurationMs: 100 }),
    )
    expect(params.min_duration_ms).toBe(100)
    expect(params.max_duration_ms).toBeUndefined()
    expect(warnings.durationSwapped).toBeUndefined()
  })

  it('handles single-bounded max only', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ maxDurationMs: 500 }),
    )
    expect(params.min_duration_ms).toBeUndefined()
    expect(params.max_duration_ms).toBe(500)
    expect(warnings.durationSwapped).toBeUndefined()
  })

  it('treats equal min/max as valid (no swap, no warning)', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ minDurationMs: 250, maxDurationMs: 250 }),
    )
    expect(params.min_duration_ms).toBe(250)
    expect(params.max_duration_ms).toBe(250)
    expect(warnings.durationSwapped).toBeUndefined()
  })
})

describe('buildFilterParams — time range', () => {
  const t0 = 1_700_000_000_000
  const t1 = 1_700_000_001_000

  it('passes start/end through unchanged when start <= end', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ startTime: t0, endTime: t1 }),
    )
    expect(params.start_time).toBe(t0)
    expect(params.end_time).toBe(t1)
    expect(warnings.timeRangeSwapped).toBeUndefined()
  })

  it('swaps start/end and flags warning when inverted', () => {
    const { params, warnings } = buildFilterParams(
      makeFilters({ startTime: t1, endTime: t0 }),
    )
    expect(params.start_time).toBe(t0)
    expect(params.end_time).toBe(t1)
    expect(warnings.timeRangeSwapped).toBe(true)
  })

  it('handles single-bounded startTime only', () => {
    const { params } = buildFilterParams(makeFilters({ startTime: t0 }))
    expect(params.start_time).toBe(t0)
    expect(params.end_time).toBeUndefined()
  })

  it('handles single-bounded endTime only', () => {
    const { params } = buildFilterParams(makeFilters({ endTime: t1 }))
    expect(params.start_time).toBeUndefined()
    expect(params.end_time).toBe(t1)
  })

  it('independently swaps duration and time-range', () => {
    const { warnings } = buildFilterParams(
      makeFilters({
        minDurationMs: 500,
        maxDurationMs: 100,
        startTime: t1,
        endTime: t0,
      }),
    )
    expect(warnings.durationSwapped).toBe(true)
    expect(warnings.timeRangeSwapped).toBe(true)
  })
})

describe('buildFilterParams — sort', () => {
  it('passes sortBy through', () => {
    expect(
      buildFilterParams(makeFilters({ sortBy: 'duration' })).params.sort_by,
    ).toBe('duration')
    expect(
      buildFilterParams(makeFilters({ sortBy: 'service_name' })).params.sort_by,
    ).toBe('service_name')
  })

  it('passes sortOrder through', () => {
    expect(
      buildFilterParams(makeFilters({ sortOrder: 'asc' })).params.sort_order,
    ).toBe('asc')
  })
})

describe('countActiveFilters', () => {
  it('returns 0 for default filters', () => {
    expect(countActiveFilters(makeFilters())).toBe(0)
  })

  it('counts each non-default field', () => {
    expect(countActiveFilters(makeFilters({ workerName: 'api' }))).toBe(1)
    expect(countActiveFilters(makeFilters({ operationName: 'GET /x' }))).toBe(1)
    expect(countActiveFilters(makeFilters({ status: 'error' }))).toBe(1)
    expect(countActiveFilters(makeFilters({ minDurationMs: 100 }))).toBe(1)
    expect(countActiveFilters(makeFilters({ maxDurationMs: 500 }))).toBe(1)
    expect(countActiveFilters(makeFilters({ startTime: 1 }))).toBe(1)
    expect(countActiveFilters(makeFilters({ endTime: 2 }))).toBe(1)
    expect(countActiveFilters(makeFilters({ attributes: [['k', 'v']] }))).toBe(
      1,
    )
    expect(countActiveFilters(makeFilters({ sortBy: 'duration' }))).toBe(1)
    expect(countActiveFilters(makeFilters({ sortOrder: 'asc' }))).toBe(1)
  })

  it('does NOT count empty attributes array', () => {
    expect(countActiveFilters(makeFilters({ attributes: [] }))).toBe(0)
  })

  it('does NOT count sortBy === start_time (default)', () => {
    expect(countActiveFilters(makeFilters({ sortBy: 'start_time' }))).toBe(0)
  })

  it('does NOT count sortOrder === desc (default)', () => {
    expect(countActiveFilters(makeFilters({ sortOrder: 'desc' }))).toBe(0)
  })

  it('sums multiple active filters', () => {
    expect(
      countActiveFilters(
        makeFilters({
          workerName: 'api',
          status: 'error',
          minDurationMs: 100,
          sortBy: 'duration',
        }),
      ),
    ).toBe(4)
  })
})
