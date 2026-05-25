import { useCallback, useEffect, useRef, useState } from 'react'
import {
  computeDockMaxWidth,
  DOCK_DEFAULT_WIDTH,
  DOCK_MIN_WIDTH,
} from '@/hooks/use-chat-dock'
import { cn } from '@/lib/utils'
import { ChatPanel } from './ChatPanel'

interface ChatDockProps {
  width: number
  onWidthChange: (next: number) => void
  collapsed: boolean
  /**
   * Bound to `Escape` while focus is on the dock chrome (not the
   * composer or any other text input). Lets keyboard users dismiss the
   * dock symmetrically with the `⌘\` open shortcut.
   */
  onCollapse: () => void
}

const clamp = (w: number, max: number) =>
  Math.max(DOCK_MIN_WIDTH, Math.min(max, w))

/**
 * Collapsible, resizable left-sticky chat panel that sits beside the active
 * route (traces / configuration / playground / examples). When expanded, the
 * width is set via a drag handle on the right edge — the dock is anchored to
 * the left, so dragging right increases the dock width, mirroring the
 * trace/span resize pattern. The dock has no fixed upper width: the drag is
 * capped by the live viewport so the route content next to the dock always
 * keeps at least `DOCK_NEIGHBOR_MIN_WIDTH` pixels visible.
 *
 * When collapsed, the dock renders nothing (no thin strip): the toggle
 * affordance lives in the global app header at the leftmost slot, matching
 * Cursor's chat-panel pattern. The previous width is preserved across
 * collapse/expand. Toggle via the header button or `⌘/Ctrl + \` (the
 * shortcut is wired inside `useChatDock` so it works even while the dock
 * is unmounted).
 */
export function ChatDock({
  width,
  onWidthChange,
  collapsed,
  onCollapse,
}: ChatDockProps) {
  const [isResizing, setIsResizing] = useState(false)
  /* Tracked in state so aria-valuemax stays accurate as the window resizes,
     even when the user isn't actively dragging. */
  const [maxWidth, setMaxWidth] = useState<number>(() => computeDockMaxWidth())
  const resizeStartRef = useRef({ x: 0, width, max: maxWidth })

  useEffect(() => {
    if (typeof window === 'undefined') return
    const onResize = () => setMaxWidth(computeDockMaxWidth())
    onResize()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const max = computeDockMaxWidth()
      resizeStartRef.current = { x: e.clientX, width, max }
      setIsResizing(true)
      document.body.style.cursor = 'col-resize'
      document.body.style.userSelect = 'none'
    },
    [width],
  )

  useEffect(() => {
    if (!isResizing) return

    const handleMouseMove = (e: MouseEvent) => {
      const dx = e.clientX - resizeStartRef.current.x
      onWidthChange(
        clamp(resizeStartRef.current.width + dx, resizeStartRef.current.max),
      )
    }

    const handleMouseUp = () => {
      setIsResizing(false)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }

    document.addEventListener('mousemove', handleMouseMove)
    document.addEventListener('mouseup', handleMouseUp)
    return () => {
      document.removeEventListener('mousemove', handleMouseMove)
      document.removeEventListener('mouseup', handleMouseUp)
    }
  }, [isResizing, onWidthChange])

  const handleReset = useCallback(() => {
    onWidthChange(DOCK_DEFAULT_WIDTH)
  }, [onWidthChange])

  if (collapsed) return null

  return (
    <>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: focus-only Esc shortcut for keyboard users; chrome remains operable by mouse */}
      <aside
        id="chat-dock"
        style={{ width }}
        onKeyDown={(e) => {
          if (e.key !== 'Escape') return
          const target = e.target as HTMLElement | null
          if (target?.isContentEditable) return
          const tag = target?.tagName
          if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return
          e.preventDefault()
          onCollapse()
        }}
        className={cn(
          'flex flex-col flex-shrink-0 h-full overflow-hidden bg-bg border-r border-rule',
          isResizing && 'select-none',
        )}
      >
        <ChatPanel density="dock" />
      </aside>
      {/* biome-ignore lint/a11y/useSemanticElements: drag handle is not a standard input; semantic separator + tabIndex is enough */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={width}
        aria-valuemin={DOCK_MIN_WIDTH}
        aria-valuemax={maxWidth}
        aria-label="resize chat dock"
        tabIndex={0}
        onMouseDown={handleMouseDown}
        onDoubleClick={handleReset}
        className={cn(
          'w-[3px] flex-shrink-0 cursor-col-resize bg-rule hover:bg-accent active:bg-accent',
          isResizing && 'bg-accent',
        )}
      />
    </>
  )
}
