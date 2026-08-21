/**
 * Pane chrome for the workspace columns: the drag-to-resize divider
 * between adjacent panes, and the screen-edge zones that grow the split.
 *
 * The edge zones live in the panes container's responsive horizontal gutter
 * (12px on narrow screens, 16px from `sm` up), so they never sit over pane
 * content or its scrollbars. Resting a mouse there for a beat — or tapping
 * it on touch — reveals the add-column affordance; clicking that adds an
 * empty pane on that side.
 */

import { Plus } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { hoverTitle } from '@/lib/keybindings/registry'
import { cn } from '@/lib/utils'

/** Mouse dwell on an edge before the add-column affordance reveals. */
const EDGE_REVEAL_MS = 200
/** Revealed affordance auto-hides after the pointer leaves it. */
const EDGE_HIDE_MS = 150
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
  /** Temporarily remove all resize interaction during panel presence motion. */
  disabled?: boolean
  /** Collapse/expand with the panel that owns this boundary. */
  motionState?: 'entering' | 'exiting'
  /** Notify the workspace controller while pointer resize owns the layout. */
  onResizeStart?: () => void
  onResizeEnd?: () => void
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
  disabled = false,
  motionState,
  onResizeStart,
  onResizeEnd,
}: ResizeHandleProps) {
  const [active, setActive] = useState(false)
  const pointerIdRef = useRef<number | null>(null)
  const startXRef = useRef(0)
  const resizeEndRef = useRef(onResizeEnd)
  resizeEndRef.current = onResizeEnd

  useEffect(
    () => () => {
      if (pointerIdRef.current === null) return
      pointerIdRef.current = null
      resizeEndRef.current?.()
    },
    [],
  )

  const finishResize = (
    element: HTMLDivElement,
    pointerId: number,
    commit = true,
  ) => {
    if (pointerIdRef.current !== pointerId) return
    // Clear ownership before release: releasing capture can synchronously
    // emit lostpointercapture, which must not commit/end the gesture twice.
    pointerIdRef.current = null
    setActive(false)
    try {
      if (element.hasPointerCapture(pointerId)) {
        element.releasePointerCapture(pointerId)
      }
    } catch {
      // already released
    }
    if (commit) onCommit()
    onResizeEnd?.()
  }

  return (
    // biome-ignore lint/a11y/useSemanticElements: a separator with slider behavior — div + ARIA is the accurate shape
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="resize panels"
      aria-valuenow={Math.round(value)}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-disabled={disabled || undefined}
      aria-hidden={motionState ? true : undefined}
      tabIndex={disabled || motionState ? -1 : 0}
      className={cn(
        'relative hidden w-1.5 shrink-0 cursor-col-resize touch-none select-none focus-visible:outline-none sm:block',
        'after:absolute after:inset-y-0 after:left-1/2 after:w-[2px] after:-translate-x-1/2 after:rounded-full after:transition-colors',
        disabled && 'pointer-events-none cursor-default',
        motionState === 'entering' &&
          'workspace-panel-divider-enter pointer-events-none',
        motionState === 'exiting' &&
          'workspace-panel-divider-exit pointer-events-none',
        active
          ? 'after:bg-accent'
          : 'after:bg-transparent hover:after:bg-edge focus-visible:after:bg-accent',
      )}
      onKeyDown={(e) => {
        if (disabled) return
        if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
        e.preventDefault()
        onResize(e.key === 'ArrowLeft' ? -KEY_STEP : KEY_STEP)
        onCommit()
      }}
      onPointerDown={(e) => {
        if (disabled) return
        if (e.button !== 0) return
        if (pointerIdRef.current !== null) return
        e.preventDefault()
        try {
          e.currentTarget.setPointerCapture(e.pointerId)
        } catch {
          return
        }
        pointerIdRef.current = e.pointerId
        startXRef.current = e.clientX
        setActive(true)
        onResizeStart?.()
      }}
      onPointerMove={(e) => {
        if (pointerIdRef.current !== e.pointerId) return
        const width = containerWidth()
        if (width <= 0) return
        const delta = (e.clientX - startXRef.current) / width
        if (delta === 0) return
        startXRef.current = e.clientX
        onResize(delta)
      }}
      onPointerUp={(e) => {
        finishResize(e.currentTarget, e.pointerId)
      }}
      onPointerCancel={(e) => {
        finishResize(e.currentTarget, e.pointerId)
      }}
      onLostPointerCapture={(e) => {
        finishResize(e.currentTarget, e.pointerId)
      }}
    />
  )
}

interface EdgeAddZoneProps {
  side: 'left' | 'right'
  onAdd: () => void
  /** Keep the indicator visible but suspend interaction during panel motion. */
  disabled?: boolean
  /** First-run discoverability: animate the persistent edge `+` and show
      the hover hint until the user grows a split once (either side). */
  nudge?: boolean
}

/**
 * The far-left / far-right sliver of the workspace. A mouse resting on it
 * reveals a GHOST PANEL — a dashed, translucent preview of the pane that
 * would appear there (a click/tap reveals immediately; touch has no hover
 * to dwell on); clicking the ghost makes it real.
 * Render while the tab is under the workspace's defensive safety ceiling.
 */
export function EdgeAddZone({
  side,
  onAdd,
  disabled = false,
  nudge = false,
}: EdgeAddZoneProps) {
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
  useEffect(() => {
    if (!disabled) return
    if (dwellRef.current != null) window.clearTimeout(dwellRef.current)
    if (hideRef.current != null) window.clearTimeout(hideRef.current)
    dwellRef.current = null
    hideRef.current = null
    setRevealed(false)
  }, [disabled])

  return (
    <div
      className={cn(
        'absolute inset-y-0 z-20 hidden pb-1.5 transition-[width] sm:flex',
        '[transition-duration:var(--motion-duration-control)] [transition-timing-function:var(--motion-ease-standard)]',
        disabled && 'pointer-events-none',
        side === 'left' ? 'left-0' : 'right-0',
        revealed ? 'w-32' : 'w-3 sm:w-4',
      )}
      onPointerEnter={(e) => {
        if (disabled) return
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
        if (disabled) return
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
          disabled={disabled}
          aria-label={`add panel on the ${side}`}
          title={
            side === 'right'
              ? hoverTitle(`add panel on the ${side}`, 'panel.split')
              : `add panel on the ${side}`
          }
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
          {nudge ? (
            <span className="px-3 text-center text-[10px] leading-relaxed text-ink-ghost">
              hover either screen edge — left or right — to add a panel
            </span>
          ) : null}
        </button>
      ) : (
        <>
          <button
            type="button"
            disabled={disabled}
            aria-label={`show the add-panel control (${side} edge)`}
            onClick={() => {
              // Touch (and impatient mice): first tap reveals, second adds.
              clearTimers()
              setRevealed(true)
            }}
            className="peer h-full w-full cursor-default bg-transparent focus-visible:outline-none"
          />
          {/* A persistent sliver of the would-be panel. Before discovery it
              shakes periodically; afterwards only the animation stops. */}
          <span
            aria-hidden
            className={cn(
              'pointer-events-none absolute inset-y-0 flex pb-1.5 text-ink-faint',
              'peer-focus-visible:-outline-offset-2 peer-focus-visible:text-accent peer-focus-visible:outline-2 peer-focus-visible:outline-accent',
              side === 'left' ? 'left-0' : 'right-0',
            )}
          >
            <span
              className={cn(
                // rounded-[3px]: the system's one-radius 6px reads pill-like
                // on a 16px sliver — deliberately tighter here.
                'flex w-3 items-center justify-center rounded-[3px] sm:w-4',
                'border border-edge bg-panel/85 backdrop-blur-[2px]',
                nudge && 'edge-nudge',
                nudge && side === 'right' && '[animation-delay:-5s]',
              )}
            >
              <Plus className="size-4 shrink-0" />
            </span>
          </span>
        </>
      )}
    </div>
  )
}
