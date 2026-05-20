import { useCallback, useEffect, useState } from 'react'

const OPEN_KEY = 'iii-chat-dock-open'
const WIDTH_KEY = 'iii-chat-dock-width'

export const DOCK_DEFAULT_WIDTH = 440
export const DOCK_MIN_WIDTH = 320
/**
 * Reserve for the route content that sits next to the dock. The dock can
 * grow as wide as the user drags, but never wider than
 * `viewportWidth - DOCK_NEIGHBOR_MIN_WIDTH` so the main route stays usable.
 */
export const DOCK_NEIGHBOR_MIN_WIDTH = 240

const FALLBACK_VIEWPORT_WIDTH = 1200

function getViewportWidth(): number {
  if (typeof window === 'undefined') return FALLBACK_VIEWPORT_WIDTH
  return window.innerWidth || FALLBACK_VIEWPORT_WIDTH
}

export function computeDockMaxWidth(viewportWidth: number = getViewportWidth()): number {
  return Math.max(DOCK_MIN_WIDTH, viewportWidth - DOCK_NEIGHBOR_MIN_WIDTH)
}

function clampWidth(w: number, viewportWidth?: number): number {
  const max = computeDockMaxWidth(viewportWidth)
  return Math.max(DOCK_MIN_WIDTH, Math.min(max, w))
}

function loadOpen(): boolean {
  if (typeof window === 'undefined') return false
  try {
    return window.localStorage.getItem(OPEN_KEY) === '1'
  } catch {
    return false
  }
}

function loadWidth(): number {
  if (typeof window === 'undefined') return DOCK_DEFAULT_WIDTH
  try {
    const raw = window.localStorage.getItem(WIDTH_KEY)
    if (!raw) return DOCK_DEFAULT_WIDTH
    const n = Number.parseInt(raw, 10)
    if (!Number.isFinite(n)) return DOCK_DEFAULT_WIDTH
    /* Only clamp by min here; the viewport-aware upper bound is enforced by
       the mount-time effect in useChatDock since window dims may differ
       between persisted-at and rehydrate-at. */
    return Math.max(DOCK_MIN_WIDTH, n)
  } catch {
    return DOCK_DEFAULT_WIDTH
  }
}

function persistOpen(value: boolean) {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(OPEN_KEY, value ? '1' : '0')
  } catch {
    // best-effort persistence
  }
}

function persistWidth(value: number) {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(WIDTH_KEY, String(value))
  } catch {
    // best-effort persistence
  }
}

export interface UseChatDockReturn {
  open: boolean
  setOpen: (value: boolean) => void
  toggle: () => void
  width: number
  setWidth: (value: number) => void
}

/**
 * Open/closed flag and resizable width for the side-by-side chat dock that
 * sits next to the traces / playground / examples routes. Both bits are
 * persisted to `localStorage` so the dock survives reloads, matching how
 * the conversation list collapse state behaves.
 *
 * The dock has no fixed upper width cap: it can be dragged as wide as the
 * user wants, with the only constraint being that the adjacent route
 * content keeps at least `DOCK_NEIGHBOR_MIN_WIDTH` pixels. If the viewport
 * shrinks below that envelope, the persisted width is re-clamped so the
 * dock doesn't overflow the screen.
 */
export function useChatDock(): UseChatDockReturn {
  const [open, setOpenState] = useState<boolean>(loadOpen)
  const [width, setWidthState] = useState<number>(loadWidth)

  useEffect(() => {
    persistOpen(open)
  }, [open])

  useEffect(() => {
    persistWidth(width)
  }, [width])

  /* Re-clamp on mount and on every viewport resize so a previously stored
     wide value (from a larger screen) shrinks to fit the current window,
     and so live resizes keep the route content visible. */
  useEffect(() => {
    if (typeof window === 'undefined') return
    const reclamp = () => {
      setWidthState((w) => clampWidth(w))
    }
    reclamp()
    window.addEventListener('resize', reclamp)
    return () => window.removeEventListener('resize', reclamp)
  }, [])

  const setOpen = useCallback((value: boolean) => {
    setOpenState(value)
  }, [])

  const toggle = useCallback(() => {
    setOpenState((v) => !v)
  }, [])

  const setWidth = useCallback((value: number) => {
    setWidthState(clampWidth(value))
  }, [])

  return { open, setOpen, toggle, width, setWidth }
}
