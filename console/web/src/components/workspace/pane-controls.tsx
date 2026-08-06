/**
 * Pane chrome for the workspace columns: the drag-to-resize divider
 * between adjacent panes, and the screen-edge zones that grow the split.
 *
 * The edge zones live in the panes container's 6px horizontal padding, so
 * they never sit over pane content (or its scrollbars). Resting a mouse
 * there for a beat — or tapping it on touch — reveals the add-column
 * affordance; clicking that adds an empty pane on that side.
 */

import { Plus } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

/** Mouse dwell on an edge before the add-column affordance reveals. */
const EDGE_REVEAL_MS = 2000
/** Revealed affordance auto-hides after the pointer leaves it. */
const EDGE_HIDE_MS = 400
/** Keyboard resize step, as a fraction of the container width. */
const KEY_STEP = 0.02

interface ResizeHandleProps {
  /** Left pane's share in percent — the separator's aria value. */
  value: number
  /** Live fraction drag: positive deltas widen the left pane. */
  onResize: (leftDelta: number) => void
  /** Gesture over — persist the current fractions. */
  onCommit: () => void
  /** Container width in px, for px→fraction conversion. */
  containerWidth: () => number
}

/**
 * The 6px gap between two panes, promoted to a col-resize grip. Deltas
 * are reported as FRACTIONS of the container width; the parent clamps
 * and re-renders, so the handle itself stays stateless. Focusable:
 * arrow keys nudge the split by 2% a press.
 */
export function ResizeHandle({
  value,
  onResize,
  onCommit,
  containerWidth,
}: ResizeHandleProps) {
  const [active, setActive] = useState(false)
  const startXRef = useRef(0)

  return (
    // biome-ignore lint/a11y/useSemanticElements: a separator with slider behavior — div + ARIA is the accurate shape
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="resize panels"
      aria-valuenow={Math.round(value)}
      aria-valuemin={0}
      aria-valuemax={100}
      tabIndex={0}
      className={cn(
        'relative w-1.5 shrink-0 cursor-col-resize touch-none select-none focus-visible:outline-none',
        'after:absolute after:inset-y-0 after:left-1/2 after:w-[2px] after:-translate-x-1/2 after:rounded-full after:transition-colors',
        active
          ? 'after:bg-accent'
          : 'after:bg-transparent hover:after:bg-rule-strong focus-visible:after:bg-accent',
      )}
      onKeyDown={(e) => {
        if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
        e.preventDefault()
        onResize(e.key === 'ArrowLeft' ? -KEY_STEP : KEY_STEP)
        onCommit()
      }}
      onPointerDown={(e) => {
        if (e.button !== 0) return
        e.preventDefault()
        e.currentTarget.setPointerCapture(e.pointerId)
        startXRef.current = e.clientX
        setActive(true)
      }}
      onPointerMove={(e) => {
        if (!active) return
        const width = containerWidth()
        if (width <= 0) return
        const delta = (e.clientX - startXRef.current) / width
        if (delta === 0) return
        startXRef.current = e.clientX
        onResize(delta)
      }}
      onPointerUp={(e) => {
        if (!active) return
        setActive(false)
        try {
          e.currentTarget.releasePointerCapture(e.pointerId)
        } catch {
          // already released
        }
        onCommit()
      }}
      onPointerCancel={() => {
        if (!active) return
        setActive(false)
        onCommit()
      }}
    />
  )
}

interface EdgeAddZoneProps {
  side: 'left' | 'right'
  onAdd: () => void
}

/**
 * The far-left / far-right sliver of the workspace. A mouse resting on it
 * reveals a GHOST PANEL — a dashed, translucent preview of the pane that
 * would appear there (a click/tap reveals immediately; touch has no hover
 * to dwell on); clicking the ghost makes it real.
 * Render only while the tab is under MAX_COLUMNS.
 */
export function EdgeAddZone({ side, onAdd }: EdgeAddZoneProps) {
  const [revealed, setRevealed] = useState(false)
  const dwellRef = useRef<number | null>(null)
  const hideRef = useRef<number | null>(null)

  const clearTimers = () => {
    if (dwellRef.current != null) window.clearTimeout(dwellRef.current)
    if (hideRef.current != null) window.clearTimeout(hideRef.current)
    dwellRef.current = null
    hideRef.current = null
  }
  useEffect(
    () => () => {
      if (dwellRef.current != null) window.clearTimeout(dwellRef.current)
      if (hideRef.current != null) window.clearTimeout(hideRef.current)
    },
    [],
  )

  return (
    <div
      className={cn(
        'absolute inset-y-0 z-20 flex pb-1.5 transition-[width] duration-150',
        side === 'left' ? 'left-0' : 'right-0',
        revealed ? 'w-32' : 'w-1.5',
      )}
      onPointerEnter={(e) => {
        if (hideRef.current != null) {
          window.clearTimeout(hideRef.current)
          hideRef.current = null
        }
        if (revealed || dwellRef.current != null) return
        if (e.pointerType === 'mouse') {
          dwellRef.current = window.setTimeout(() => {
            dwellRef.current = null
            setRevealed(true)
          }, EDGE_REVEAL_MS)
        }
      }}
      onPointerLeave={() => {
        if (dwellRef.current != null) {
          window.clearTimeout(dwellRef.current)
          dwellRef.current = null
        }
        if (revealed) {
          hideRef.current = window.setTimeout(() => {
            hideRef.current = null
            setRevealed(false)
          }, EDGE_HIDE_MS)
        }
      }}
    >
      {revealed ? (
        // The ghost panel: same shape as a real pane (rounded, bordered,
        // bottom-aligned with the row), but dashed and translucent — a
        // preview of the pane the click will create.
        <button
          type="button"
          aria-label={`add panel on the ${side}`}
          title={`add panel on the ${side}`}
          onClick={() => {
            clearTimers()
            setRevealed(false)
            onAdd()
          }}
          className={cn(
            'flex h-full w-full flex-col items-center justify-center gap-2 rounded-sm',
            'border border-dashed border-ink-ghost bg-panel/60 backdrop-blur-[2px]',
            'font-mono text-[11px] lowercase text-ink-faint',
            'mx-1.5 hover:border-accent hover:text-ink transition-colors',
          )}
        >
          <Plus className="size-4" />
          new panel
        </button>
      ) : (
        <button
          type="button"
          aria-label={`show the add-panel control (${side} edge)`}
          onClick={() => {
            // Touch (and impatient mice): first tap reveals, second adds.
            clearTimers()
            setRevealed(true)
          }}
          className="h-full w-full cursor-default bg-transparent focus-visible:bg-accent-muted focus-visible:outline-none"
        />
      )}
    </div>
  )
}
