import { describe, expect, it } from 'vitest'
import {
  computeDockDefaultWidth,
  computeDockMaxWidth,
  DOCK_MIN_WIDTH,
} from './use-chat-dock'

describe('chat dock sizing', () => {
  it('starts with an even viewport split', () => {
    expect(computeDockDefaultWidth(1280)).toBe(640)
    expect(computeDockDefaultWidth(1920)).toBe(960)
  })

  it('keeps the minimum widths when the viewport is narrow', () => {
    expect(computeDockDefaultWidth(500)).toBe(DOCK_MIN_WIDTH)
    expect(computeDockMaxWidth(500)).toBe(DOCK_MIN_WIDTH)
  })
})
