import { useCallback, useEffect, useState } from 'react'

const WIDTH_KEY = 'iii-chat-dock-width'
const COLLAPSED_KEY = 'iii-chat-dock-collapsed'
// Legacy: when the dock was first made toggleable we persisted an open/closed
// flag under a different key. The collapse state now lives under
// COLLAPSED_KEY; sweep the old key on first run so it doesn't linger in
// users' localStorage forever.
const LEGACY_OPEN_KEY = 'iii-chat-dock-open'

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

export function computeDockMaxWidth(
  viewportWidth: number = getViewportWidth(),
): number {
  return Math.max(DOCK_MIN_WIDTH, viewportWidth - DOCK_NEIGHBOR_MIN_WIDTH)
}

/** Start with an even split between the chat dock and the active route. */
export function computeDockDefaultWidth(
  viewportWidth: number = getViewportWidth(),
): number {
  return clampWidth(viewportWidth / 2, viewportWidth)
}

function clampWidth(w: number, viewportWidth?: number): number {
  const max = computeDockMaxWidth(viewportWidth)
  return Math.max(DOCK_MIN_WIDTH, Math.min(max, w))
}

function loadWidth(): number {
  if (typeof window === 'undefined') return computeDockDefaultWidth()
  try {
    const raw = window.localStorage.getItem(WIDTH_KEY)
    if (!raw) return computeDockDefaultWidth()
    const n = Number.parseInt(raw, 10)
    if (!Number.isFinite(n)) return computeDockDefaultWidth()
    /* Only clamp by min here; the viewport-aware upper bound is enforced by
       the mount-time effect in useChatDock since window dims may differ
       between persisted-at and rehydrate-at. */
    return Math.max(DOCK_MIN_WIDTH, n)
  } catch {
    return computeDockDefaultWidth()
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

function sweepLegacyOpenKey(): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.removeItem(LEGACY_OPEN_KEY)
  } catch {
    // best-effort
  }
}

function loadCollapsed(): boolean {
  if (typeof window === 'undefined') return false
  try {
    return window.localStorage.getItem(COLLAPSED_KEY) === '1'
  } catch {
    return false
  }
}

function persistCollapsed(value: boolean): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(COLLAPSED_KEY, value ? '1' : '0')
  } catch {
    // best-effort persistence
  }
}

export interface UseChatDockReturn {
  width: number
  setWidth: (value: number) => void
  collapsed: boolean
  setCollapsed: (value: boolean) => void
  toggleCollapsed: () => void
}

/**
 * Resizable, collapsible chat dock that sits next to the active route pane.
 * Width and collapsed state are both persisted to `localStorage` so the dock
 * survives reloads at whatever size and visibility the user left it in.
 *
 * The dock has no fixed upper width cap: it can be dragged as wide as the
 * user wants, with the only constraint being that the adjacent route
 * content keeps at least `DOCK_NEIGHBOR_MIN_WIDTH` pixels. If the viewport
 * shrinks below that envelope, the persisted width is re-clamped so the
 * dock doesn't overflow the screen. When collapsed, the dock renders as a
 * thin strip with a re-expand affordance — the previous width is preserved
 * and restored on re-expand.
 */
export function useChatDock(): UseChatDockReturn {
  const [width, setWidthState] = useState<number>(loadWidth)
  const [collapsed, setCollapsedState] = useState<boolean>(loadCollapsed)

  useEffect(() => {
    persistWidth(width)
  }, [width])

  useEffect(() => {
    persistCollapsed(collapsed)
  }, [collapsed])

  /* One-shot cleanup of the legacy open/closed flag from a prior iteration
     of the dock. Idempotent: removeItem on a missing key is a no-op. */
  useEffect(() => {
    sweepLegacyOpenKey()
  }, [])

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

  const setWidth = useCallback((value: number) => {
    setWidthState(clampWidth(value))
  }, [])

  const setCollapsed = useCallback((value: boolean) => {
    setCollapsedState(value)
  }, [])

  const toggleCollapsed = useCallback(() => {
    setCollapsedState((v) => !v)
  }, [])

  /* ⌘\ / Ctrl+\ toggles the dock. Listener lives in the hook so the
     shortcut keeps working when the dock is collapsed and the panel
     itself isn't mounted. Ignored when the user is typing into an
     editable element so we don't fight composer input. */
  useEffect(() => {
    if (typeof window === 'undefined') return
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== '\\') return
      if (!(e.metaKey || e.ctrlKey)) return
      const target = e.target as HTMLElement | null
      if (target?.isContentEditable) return
      const tag = target?.tagName
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
      e.preventDefault()
      toggleCollapsed()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [toggleCollapsed])

  return { width, setWidth, collapsed, setCollapsed, toggleCollapsed }
}
