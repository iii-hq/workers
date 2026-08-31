/**
 * The terminal font size, shared by every console terminal in this repo:
 * shell's docked panes and the agent-CLI pages (claude, pi).
 *
 * It lives in `localStorage` rather than in a worker's configuration entry,
 * because it is a property of the person reading the screen, not of the
 * machine running the session: the same engine viewed from a laptop and from
 * a wall display wants two different answers. Every terminal in every open
 * tab follows one value — a `storage` event carries it between tabs, a
 * `CustomEvent` carries it within one.
 */

import { useCallback, useSyncExternalStore } from 'react'

export const MIN_FONT_SIZE = 8
export const MAX_FONT_SIZE = 40
/** 13px was the old hard-coded size and reads small on a dense display. */
export const DEFAULT_FONT_SIZE = 14

const STORAGE_KEY = 'iii::terminal::font-size'
const CHANGE_EVENT = 'iii:terminal-font-size'

/** Whole pixels inside the range; anything unreadable becomes the default. */
export function clampFontSize(value: unknown): number {
  const size = typeof value === 'string' ? Number.parseInt(value, 10) : Number(value)
  if (!Number.isFinite(size)) return DEFAULT_FONT_SIZE
  return Math.min(MAX_FONT_SIZE, Math.max(MIN_FONT_SIZE, Math.round(size)))
}

/** Storage is absent under SSR and blocked in some privacy modes. */
function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage
  } catch {
    return null
  }
}

export function readFontSize(): number {
  const stored = storage()?.getItem(STORAGE_KEY)
  return stored === null || stored === undefined ? DEFAULT_FONT_SIZE : clampFontSize(stored)
}

/** Persists the clamped size, tells every listener, and returns what it wrote. */
export function writeFontSize(value: unknown): number {
  const size = clampFontSize(value)
  try {
    storage()?.setItem(STORAGE_KEY, String(size))
  } catch {
    // Full or blocked: the size still applies to this page, it just will not
    // survive a reload.
  }
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(CHANGE_EVENT, { detail: size }))
  }
  return size
}

/** Fires on a change from this tab and from any other tab. */
export function subscribeFontSize(onChange: () => void): () => void {
  if (typeof window === 'undefined') return () => {}
  const fromOtherTab = (event: StorageEvent) => {
    if (event.key === null || event.key === STORAGE_KEY) onChange()
  }
  window.addEventListener(CHANGE_EVENT, onChange)
  window.addEventListener('storage', fromOtherTab)
  return () => {
    window.removeEventListener(CHANGE_EVENT, onChange)
    window.removeEventListener('storage', fromOtherTab)
  }
}

/**
 * The size and a setter. Every terminal that calls this re-renders together,
 * so one pane's `A+` resizes the pane beside it.
 */
export function useTerminalFontSize(): [number, (value: unknown) => void] {
  const size = useSyncExternalStore(subscribeFontSize, readFontSize, () => DEFAULT_FONT_SIZE)
  const setSize = useCallback((value: unknown) => {
    writeFontSize(value)
  }, [])
  return [size, setSize]
}

/** One step of the stepper, clamped — `+`/`−` buttons and `Ctrl`+wheel share it. */
export function stepFontSize(current: number, direction: 1 | -1): number {
  return clampFontSize(current + direction)
}
