import { describe, expect, it } from 'vitest'
import {
  canGoBack,
  canGoForward,
  EMPTY_HISTORY,
  forgetPath,
  goBack,
  goForward,
  pushLocation,
  updateCurrentLine,
  recentPaths,
} from '../nav-history'

describe('nav-history', () => {
  it('pushes, goes back and forward', () => {
    let h = pushLocation(EMPTY_HISTORY, { path: 'a.ts' })
    h = pushLocation(h, { path: 'b.ts', line: 4 })
    expect(canGoBack(h)).toBe(true)
    expect(canGoForward(h)).toBe(false)
    const back = goBack(h)
    expect(back.location).toEqual({ path: 'a.ts' })
    expect(canGoForward(back.history)).toBe(true)
    const fwd = goForward(back.history)
    expect(fwd.location).toEqual({ path: 'b.ts', line: 4 })
  })

  it('a new push after going back drops the forward branch', () => {
    let h = pushLocation(EMPTY_HISTORY, { path: 'a.ts' })
    h = pushLocation(h, { path: 'b.ts' })
    h = goBack(h).history
    h = pushLocation(h, { path: 'c.ts' })
    expect(h.entries.map((e) => e.path)).toEqual(['a.ts', 'c.ts'])
    expect(canGoForward(h)).toBe(false)
  })

  it('ignores an identical consecutive location', () => {
    let h = pushLocation(EMPTY_HISTORY, { path: 'a.ts' })
    const same = pushLocation(h, { path: 'a.ts' })
    expect(same).toBe(h)
    h = updateCurrentLine(h, 12)
    expect(h.entries[0]).toEqual({ path: 'a.ts', line: 12 })
  })

  it('forgets a closed path and keeps the index sensible', () => {
    let h = pushLocation(EMPTY_HISTORY, { path: 'a.ts' })
    h = pushLocation(h, { path: 'b.ts' })
    h = pushLocation(h, { path: 'c.ts' })
    h = goBack(h).history // on b
    h = forgetPath(h, 'b.ts')
    expect(h.entries.map((e) => e.path)).toEqual(['a.ts', 'c.ts'])
    expect(h.index).toBe(0)
    expect(forgetPath(h, 'zzz')).toBe(h)
  })
})

describe('recentPaths', () => {
  it('lists distinct files, newest first, up to the limit', () => {
    let history = EMPTY_HISTORY
    for (const path of ['a.ts', 'b.ts', 'a.ts', 'c.ts', 'b.ts']) history = pushLocation(history, { path })
    expect(recentPaths(history, 10)).toEqual(['b.ts', 'c.ts', 'a.ts'])
    expect(recentPaths(history, 2)).toEqual(['b.ts', 'c.ts'])
    expect(recentPaths(EMPTY_HISTORY, 5)).toEqual([])
  })
})
