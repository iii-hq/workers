import { useCallback, useEffect, useState } from 'react'

const WIDTH_KEY = 'iii-chat-sidebar-width'

export const SIDEBAR_DEFAULT_WIDTH = 220
export const SIDEBAR_MIN_WIDTH = 160
/**
 * ponytail: fixed cap rather than measuring the dock. The sidebar sits inside
 * the chat dock (default 440px) next to ChatView (flex-1); a 420 ceiling keeps
 * ChatView visible at the default dock width. If the user wants more list, they
 * widen the dock too. Upgrade to a measured `dockWidth - CHATVIEW_MIN` clamp
 * only if that coupling proves annoying.
 */
export const SIDEBAR_MAX_WIDTH = 420

export function clampSidebarWidth(w: number): number {
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, w))
}

function loadWidth(): number {
  if (typeof window === 'undefined') return SIDEBAR_DEFAULT_WIDTH
  try {
    const raw = window.localStorage.getItem(WIDTH_KEY)
    if (!raw) return SIDEBAR_DEFAULT_WIDTH
    const n = Number.parseInt(raw, 10)
    if (!Number.isFinite(n)) return SIDEBAR_DEFAULT_WIDTH
    return clampSidebarWidth(n)
  } catch {
    return SIDEBAR_DEFAULT_WIDTH
  }
}

function persistWidth(value: number): void {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(WIDTH_KEY, String(value))
  } catch {
    // best-effort persistence
  }
}

export interface UseSidebarWidthReturn {
  width: number
  setWidth: (value: number) => void
}

/**
 * Width of the conversation list inside the chat dock, persisted to
 * `localStorage`. Mirrors `use-chat-dock` but width-only: the collapse toggle
 * already lives in `ChatPanel`, and the list has no global keyboard shortcut.
 */
export function useSidebarWidth(): UseSidebarWidthReturn {
  const [width, setWidthState] = useState<number>(loadWidth)

  useEffect(() => {
    persistWidth(width)
  }, [width])

  const setWidth = useCallback((value: number) => {
    setWidthState(clampSidebarWidth(value))
  }, [])

  return { width, setWidth }
}
