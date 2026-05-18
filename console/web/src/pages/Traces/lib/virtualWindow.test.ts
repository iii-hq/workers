import { describe, expect, it } from 'vitest'
import { computeVirtualWindow } from './virtualWindow'

const ROW = 32

describe('computeVirtualWindow', () => {
  it('returns an empty window when the list is empty', () => {
    expect(
      computeVirtualWindow({
        scrollTop: 0,
        containerHeight: 800,
        rowHeight: ROW,
        itemCount: 0,
      }),
    ).toEqual({ startIndex: 0, endIndex: 0, offsetY: 0 })
  })

  it('returns a first-paint window before the container has been measured', () => {
    // containerHeight === 0 is the state on the very first render, before
    // the resize observer fires. We render a minimal top-of-list window
    // so the user sees content instead of an empty scroll surface.
    const win = computeVirtualWindow({
      scrollTop: 0,
      containerHeight: 0,
      rowHeight: ROW,
      itemCount: 100,
    })
    expect(win.startIndex).toBe(0)
    expect(win.endIndex).toBeGreaterThan(0)
    expect(win.endIndex).toBeLessThanOrEqual(100)
    expect(win.offsetY).toBe(0)
  })

  it('renders the full list when it is shorter than the viewport', () => {
    // 5 items, 32px row, 800px viewport. Overscan would push past the end;
    // endIndex must clamp to itemCount and offsetY stays 0.
    const win = computeVirtualWindow({
      scrollTop: 0,
      containerHeight: 800,
      rowHeight: ROW,
      itemCount: 5,
    })
    expect(win.startIndex).toBe(0)
    expect(win.endIndex).toBe(5)
    expect(win.offsetY).toBe(0)
  })

  it('returns a window around the scroll position with overscan padding', () => {
    // scroll 1000px down a 4000-item list (~128k content), 800px viewport,
    // 32px rows, default overscan 8.
    // Center range: floor(1000/32)=31  ..  ceil((1000+800)/32)=57
    // With overscan 8: start=23, end=65, offset=23*32=736.
    const win = computeVirtualWindow({
      scrollTop: 1000,
      containerHeight: 800,
      rowHeight: ROW,
      itemCount: 4000,
    })
    expect(win.startIndex).toBe(23)
    expect(win.endIndex).toBe(65)
    expect(win.offsetY).toBe(23 * ROW)
  })

  it('clamps the window when scrolled past the end of the list', () => {
    // scrollTop overshoots — browser bounce, programmatic scroll, or list
    // shrunk while scrolled. endIndex must not exceed itemCount, and
    // startIndex must stay inside the list so offsetY is valid.
    const win = computeVirtualWindow({
      scrollTop: 1_000_000,
      containerHeight: 800,
      rowHeight: ROW,
      itemCount: 100,
    })
    expect(win.startIndex).toBeGreaterThanOrEqual(0)
    expect(win.startIndex).toBeLessThanOrEqual(100)
    expect(win.endIndex).toBe(100)
    expect(win.endIndex).toBeGreaterThanOrEqual(win.startIndex)
  })

  it('clamps a negative scrollTop to 0 (browser overshoot / bounce)', () => {
    const win = computeVirtualWindow({
      scrollTop: -200,
      containerHeight: 800,
      rowHeight: ROW,
      itemCount: 100,
    })
    expect(win.startIndex).toBe(0)
    expect(win.offsetY).toBe(0)
    expect(win.endIndex).toBeGreaterThan(0)
  })

  it('respects a custom overscan', () => {
    // Same mid-scroll inputs as above but overscan=2 instead of 8.
    // Center range: 31..57. With overscan 2: start=29, end=59.
    const win = computeVirtualWindow({
      scrollTop: 1000,
      containerHeight: 800,
      rowHeight: ROW,
      itemCount: 4000,
      overscan: 2,
    })
    expect(win.startIndex).toBe(29)
    expect(win.endIndex).toBe(59)
  })

  it('returns an empty window when rowHeight is invalid', () => {
    expect(
      computeVirtualWindow({
        scrollTop: 0,
        containerHeight: 800,
        rowHeight: 0,
        itemCount: 100,
      }),
    ).toEqual({ startIndex: 0, endIndex: 0, offsetY: 0 })
  })
})
