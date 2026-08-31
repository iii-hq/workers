import { describe, expect, it } from 'vitest'
import { dividerForPanel, panelMotionDirection } from './panel-motion'

describe('workspace panel motion planning', () => {
  it('pairs the first panel with its following divider and others with their preceding divider', () => {
    expect(dividerForPanel(0, 3)).toBe(1)
    expect(dividerForPanel(1, 3)).toBe(1)
    expect(dividerForPanel(2, 3)).toBe(2)
    expect(dividerForPanel(0, 1)).toBeNull()
  })

  it('moves panels toward their nearest edge', () => {
    expect(panelMotionDirection(0, 3)).toBe('left')
    expect(panelMotionDirection(1, 3)).toBe('right')
    expect(panelMotionDirection(2, 3)).toBe('right')
  })
})
