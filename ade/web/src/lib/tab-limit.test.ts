import { describe, expect, it, vi } from 'vitest'
import {
  canCreateTab,
  createTabOrNotice,
  DEFAULT_TAB_LIMIT,
  MAX_TAB_LIMIT,
  MIN_TAB_LIMIT,
  onTabLimitNotice,
  parseTabLimit,
  tabLimitReachedTitle,
} from './tab-limit'

describe('parseTabLimit', () => {
  it('falls back to the default when the entry says nothing', () => {
    expect(parseTabLimit(null)).toBe(DEFAULT_TAB_LIMIT)
    expect(parseTabLimit(undefined)).toBe(DEFAULT_TAB_LIMIT)
    expect(parseTabLimit({})).toBe(DEFAULT_TAB_LIMIT)
    expect(parseTabLimit({ tabs: {} })).toBe(DEFAULT_TAB_LIMIT)
  })

  it('reads a configured limit', () => {
    expect(parseTabLimit({ tabs: { limit: 5 } })).toBe(5)
    expect(parseTabLimit({ tabs: { limit: 1 } })).toBe(1)
  })

  it('clamps to the schema range and floors fractions', () => {
    expect(parseTabLimit({ tabs: { limit: 0 } })).toBe(MIN_TAB_LIMIT)
    expect(parseTabLimit({ tabs: { limit: -3 } })).toBe(MIN_TAB_LIMIT)
    expect(parseTabLimit({ tabs: { limit: 999 } })).toBe(MAX_TAB_LIMIT)
    expect(parseTabLimit({ tabs: { limit: 7.9 } })).toBe(7)
  })

  it('treats a hand-edited entry that is not a number as the default', () => {
    expect(parseTabLimit({ tabs: { limit: '12' } })).toBe(DEFAULT_TAB_LIMIT)
    expect(parseTabLimit({ tabs: { limit: null } })).toBe(DEFAULT_TAB_LIMIT)
    expect(parseTabLimit({ tabs: { limit: Number.NaN } })).toBe(
      DEFAULT_TAB_LIMIT,
    )
    expect(parseTabLimit({ tabs: 'nope' })).toBe(DEFAULT_TAB_LIMIT)
  })
})

describe('canCreateTab', () => {
  it('allows one more tab until the count reaches the limit', () => {
    expect(canCreateTab(0, 1)).toBe(true)
    expect(canCreateTab(1, 1)).toBe(false)
    expect(canCreateTab(11, 12)).toBe(true)
    expect(canCreateTab(12, 12)).toBe(false)
  })

  it('refuses when the count already exceeds a lowered limit', () => {
    expect(canCreateTab(15, 12)).toBe(false)
  })
})

describe('tabLimitReachedTitle', () => {
  it('names the limit and the way out', () => {
    const title = tabLimitReachedTitle(12)
    expect(title).toContain('12')
    expect(title).toMatch(/close a tab/i)
    expect(title).toMatch(/configuration/i)
  })
})

// QA 2026-11-16 F1: ⌃T at the limit did nothing and said nothing.
describe('createTabOrNotice', () => {
  it('opens a workspace under the limit, and says why not at it', () => {
    const heard = vi.fn()
    const stop = onTabLimitNotice(heard)
    const createTab = vi.fn()
    expect(createTabOrNotice({ canCreateTab: true, createTab })).toBe(true)
    expect(createTab).toHaveBeenCalledWith({ columns: 1 })
    expect(heard).not.toHaveBeenCalled()
    createTab.mockClear()
    expect(createTabOrNotice({ canCreateTab: false, createTab })).toBe(false)
    expect(createTab).not.toHaveBeenCalled()
    expect(heard).toHaveBeenCalledTimes(1)
    stop()
  })
})
