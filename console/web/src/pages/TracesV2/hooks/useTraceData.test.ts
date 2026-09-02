import { describe, expect, it } from 'vitest'
import {
  buildTraceListRequestParams,
  filterHiddenTraceRows,
  hiddenFunctionsKey,
  mergeNewTraceIds,
  shouldDeferTraceUpdate,
  type TraceListItem,
  traceTotalForResponse,
} from './useTraceData'

describe('mergeNewTraceIds', () => {
  it('keeps rows still flashing when the next answer adds more', () => {
    const merged = mergeNewTraceIds(
      new Set(['a']),
      new Set(['b']),
      new Set(['a', 'b', 'c']),
    )
    expect([...merged].sort()).toEqual(['a', 'b'])
  })

  it('drops ids that left the page so the set stays bounded', () => {
    const merged = mergeNewTraceIds(
      new Set(['gone', 'a']),
      new Set(['b']),
      new Set(['a', 'b']),
    )
    expect([...merged].sort()).toEqual(['a', 'b'])
  })
})

describe('shouldDeferTraceUpdate', () => {
  it('renders the first answer of an empty scope while hovered', () => {
    expect(shouldDeferTraceUpdate(true, false)).toBe(false)
  })

  it('freezes updates to an existing list while hovered', () => {
    expect(shouldDeferTraceUpdate(true, true)).toBe(true)
  })

  it('renders updates immediately when the list is not hovered', () => {
    expect(shouldDeferTraceUpdate(false, true)).toBe(false)
  })
})

describe('buildTraceListRequestParams', () => {
  it('preserves the requested server page and projects only requested attributes', () => {
    expect(
      buildTraceListRequestParams({
        filterParams: { offset: 100, limit: 50, status: 'error' },
        showSystem: false,
        debouncedSearch: '',
        hiddenFunctions: undefined,
        attributeProjection: ['custom.label'],
      }),
    ).toMatchObject({
      offset: 100,
      limit: 50,
      status: 'error',
      include_internal: false,
      attribute_projection: ['custom.label'],
    })
  })

  it('adds child-span search and root exclusions without replacing pagination', () => {
    expect(
      buildTraceListRequestParams({
        filterParams: { offset: 50, limit: 25 },
        showSystem: true,
        debouncedSearch: 'old failure',
        hiddenFunctions: ['noisy::heartbeat'],
        attributeProjection: undefined,
      }),
    ).toMatchObject({
      offset: 50,
      limit: 25,
      name: 'old failure',
      search_all_spans: true,
      include_internal: true,
      exclude_attributes: [
        ['faas.invoked_name', 'noisy::heartbeat'],
        ['function_id', 'noisy::heartbeat'],
      ],
    })
  })
})

describe('legacy hidden-function compatibility', () => {
  const trace = (functionId: string): TraceListItem => ({
    traceId: functionId,
    rootOperation: functionId,
    functionId,
    status: 'ok',
    startTime: 1,
    spanCount: 1,
    workers: [],
  })

  it('preserves commas inside function IDs', () => {
    const key = hiddenFunctionsKey(['worker::function,variant'])
    const rows = [trace('worker::function,variant'), trace('worker::function')]

    expect(filterHiddenTraceRows(rows, key)).toEqual([
      trace('worker::function'),
    ])
  })

  it('uses the locally filtered total only for legacy responses', () => {
    const base = { traces: [], total: 50, offset: 0, limit: 50 }

    expect(traceTotalForResponse(base, 2)).toBe(50)
    expect(traceTotalForResponse({ ...base, legacyContract: true }, 2)).toBe(2)
  })
})
