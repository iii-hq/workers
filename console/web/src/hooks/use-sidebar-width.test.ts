import { describe, expect, it } from 'vitest'
import {
  clampSidebarWidth,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from './use-sidebar-width'

describe('clampSidebarWidth', () => {
  it('clamps below the minimum up to the minimum', () => {
    expect(clampSidebarWidth(0)).toBe(SIDEBAR_MIN_WIDTH)
    expect(clampSidebarWidth(SIDEBAR_MIN_WIDTH - 50)).toBe(SIDEBAR_MIN_WIDTH)
  })

  it('clamps above the maximum down to the maximum', () => {
    expect(clampSidebarWidth(9999)).toBe(SIDEBAR_MAX_WIDTH)
    expect(clampSidebarWidth(SIDEBAR_MAX_WIDTH + 50)).toBe(SIDEBAR_MAX_WIDTH)
  })

  it('passes through an in-range width unchanged', () => {
    const mid = Math.round((SIDEBAR_MIN_WIDTH + SIDEBAR_MAX_WIDTH) / 2)
    expect(clampSidebarWidth(mid)).toBe(mid)
  })
})
