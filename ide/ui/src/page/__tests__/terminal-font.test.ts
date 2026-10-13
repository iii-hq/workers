import {
  clampFontSize,
  DEFAULT_FONT_SIZE,
  MAX_FONT_SIZE,
  MIN_FONT_SIZE,
  readFontSize,
  stepFontSize,
  subscribeFontSize,
  writeFontSize,
} from '@iii-workers/terminal-font'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/** The shared store is a browser global; the tests own a fake one. */
function fakeStorage(): Storage {
  const map = new Map<string, string>()
  return {
    get length() {
      return map.size
    },
    clear: () => map.clear(),
    getItem: (key: string) => map.get(key) ?? null,
    key: (index: number) => [...map.keys()][index] ?? null,
    removeItem: (key: string) => void map.delete(key),
    setItem: (key: string, value: string) => void map.set(key, value),
  } as Storage
}

beforeEach(() => {
  vi.stubGlobal('window', {
    localStorage: fakeStorage(),
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => true,
  })
})

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('terminal font size', () => {
  it('holds the size inside the readable range', () => {
    expect(clampFontSize(MIN_FONT_SIZE - 5)).toBe(MIN_FONT_SIZE)
    expect(clampFontSize(MAX_FONT_SIZE + 100)).toBe(MAX_FONT_SIZE)
    expect(clampFontSize(17.6)).toBe(18)
    expect(clampFontSize('20')).toBe(20)
  })

  it('falls back to the default rather than rendering nothing', () => {
    // An empty store, a hand-edited value, a key from another product.
    expect(readFontSize()).toBe(DEFAULT_FONT_SIZE)
    expect(clampFontSize('not a number')).toBe(DEFAULT_FONT_SIZE)
    expect(clampFontSize(undefined)).toBe(DEFAULT_FONT_SIZE)
  })

  it('persists what it clamped, not what it was given', () => {
    expect(writeFontSize(99)).toBe(MAX_FONT_SIZE)
    expect(readFontSize()).toBe(MAX_FONT_SIZE)
    expect(writeFontSize(16)).toBe(16)
    expect(readFontSize()).toBe(16)
  })

  it('steps one pixel at a time and stops at the ends', () => {
    expect(stepFontSize(14, 1)).toBe(15)
    expect(stepFontSize(14, -1)).toBe(13)
    expect(stepFontSize(MIN_FONT_SIZE, -1)).toBe(MIN_FONT_SIZE)
    expect(stepFontSize(MAX_FONT_SIZE, 1)).toBe(MAX_FONT_SIZE)
  })

  it('survives a browser that refuses storage', () => {
    vi.stubGlobal('window', {
      get localStorage(): Storage {
        throw new Error('blocked')
      },
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => true,
    })
    // The size still applies to the live page; only the memory of it is lost.
    expect(readFontSize()).toBe(DEFAULT_FONT_SIZE)
    expect(writeFontSize(22)).toBe(22)
  })

  it('reads storage once, then follows this tab and the others', () => {
    const key = 'iii::terminal::font-size'
    const store = fakeStorage()
    const getItem = vi.spyOn(store, 'getItem')
    const listeners = new Map<string, (event: unknown) => void>()
    vi.stubGlobal('window', {
      localStorage: store,
      addEventListener: (name: string, listener: (event: unknown) => void) =>
        listeners.set(name, listener),
      removeEventListener: () => {},
      dispatchEvent: () => true,
    })
    store.setItem(key, '18')

    const stop = subscribeFontSize(() => {})
    expect(readFontSize()).toBe(18)
    expect(readFontSize()).toBe(18)
    expect(getItem).toHaveBeenCalledTimes(1)
    // Another tab stored a new size.
    store.setItem(key, '22')
    listeners.get('storage')?.({ key })
    expect(readFontSize()).toBe(22)
    // Another page's copy of this module changed it in this tab.
    listeners.get('iii:terminal-font-size')?.({ detail: 30 })
    expect(readFontSize()).toBe(30)
    stop()
  })

  it('reads storage again while nothing here listens', () => {
    const key = 'iii::terminal::font-size'
    const store = fakeStorage()
    vi.stubGlobal('window', {
      localStorage: store,
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => true,
    })
    store.setItem(key, '18')
    expect(readFontSize()).toBe(18)
    // Another page's copy of this module stored a size while no terminal
    // here was mounted to hear it, then one mounts and subscribes.
    store.setItem(key, '24')
    const stop = subscribeFontSize(() => {})
    expect(readFontSize()).toBe(24)
    stop()
    // And again after the last one unmounted.
    store.setItem(key, '30')
    expect(readFontSize()).toBe(30)
  })

  it('unsubscribes cleanly', () => {
    const listeners: string[] = []
    vi.stubGlobal('window', {
      localStorage: fakeStorage(),
      addEventListener: (name: string) => listeners.push(name),
      removeEventListener: (name: string) => {
        listeners.splice(listeners.indexOf(name), 1)
      },
      dispatchEvent: () => true,
    })
    const stop = subscribeFontSize(() => {})
    expect(listeners).toHaveLength(2)
    stop()
    expect(listeners).toHaveLength(0)
  })
})
