import { describe, expect, it } from 'vitest'
import {
  buildTraceListRequestParams,
  shouldDeferTraceUpdate,
} from './useTraceData'

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
