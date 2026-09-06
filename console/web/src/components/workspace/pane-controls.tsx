/**
 * Pane chrome for the workspace columns: the drag-to-resize divider
 * between adjacent panes, and the screen-edge zones that grow the split.
 *
 * The edge zones live in the panes container's responsive horizontal gutter
 * (12px on narrow screens, 16px from `sm` up), so they never sit over pane
 * content or its scrollbars. Resting a mouse there for a beat — or tapping
 * it on touch — reveals a preview of the pane a click would add on that
 * side; until the first split the edge also keeps a framed `+` sliver.
 */

import { GripHorizontal, Plus } from 'lucide-react'
import {
  type DragEvent,
  type FocusEvent,
  type KeyboardEvent,
  type MutableRefObject,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from 'react'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { REDUCED_MOTION_QUERY, useMediaQuery } from '@/hooks/use-media-query'
import { bindingsFor, hoverTitle } from '@/lib/keybindings/registry'
import { cn } from '@/lib/utils'

/** Mouse dwell on an edge before the split preview reveals. */
const EDGE_REVEAL_MS = 200
/** Revealed preview auto-hides after the pointer leaves it. */
const EDGE_HIDE_MS = 150
/** The preview's exit motion — the `fast` duration token. */
const EDGE_EXIT_MS = 120
/** Existing columns the split schematic draws; more would not fit the card. */
const SCHEMATIC_MAX = 3
/** Keyboard resize step, as a fraction of the container width. */
const KEY_STEP = 0.02

interface PanelDragHandleProps {
  index: number
  count: number
  disabled?: boolean
  dragging?: boolean
  onDragStart: (event: DragEvent<HTMLButtonElement>) => void
  onDragEnd: () => void
  onMove: (nextIndex: number) => void
}

/**
 * Desktop-only panel reorder handle. Native drag keeps the gesture isolated
 * from the hosted page, while arrow/Home/End keys offer the same operation
 * without a pointer.
 */
export function PanelDragHandle({
  index,
  count,
  disabled = false,
  dragging = false,
  onDragStart,
  onDragEnd,
  onMove,
}: PanelDragHandleProps) {
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return
    let next: number | null = null
    if (event.key === 'ArrowLeft' && index > 0) next = index - 1
    else if (event.key === 'ArrowRight' && index < count - 1) next = index + 1
    else if (event.key === 'Home' && index > 0) next = 0
    else if (event.key === 'End' && index < count - 1) next = count - 1
    if (next === null) return
    event.preventDefault()
    onMove(next)
  }

  return (
    <button
      type="button"
      draggable={!disabled}
      disabled={disabled}
      aria-label={`reorder panel ${index + 1}`}
      aria-keyshortcuts="ArrowLeft ArrowRight Home End"
      title="drag to reorder (arrow keys also work)"
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onKeyDown={onKeyDown}
      className={cn(
        'absolute top-0 left-1/2 z-30 hidden h-5 w-10 -translate-x-1/2 items-center justify-center rounded-b-sm border-x border-b border-edge bg-panel text-ink-ghost shadow-sm sm:flex',
        'cursor-grab opacity-40 transition-[color,opacity,background-color] hover:bg-surface-hover hover:text-ink hover:opacity-100 focus-visible:bg-surface-hover focus-visible:text-ink focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent active:cursor-grabbing',
        disabled && 'cursor-default opacity-0',
        dragging && 'cursor-grabbing bg-surface-hover text-ink opacity-100',
      )}
    >
      <GripHorizontal aria-hidden className="size-4" />
    </button>
  )
}

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

type PreviewPhase = 'idle' | 'open' | 'closing'

function clearTimer(ref: MutableRefObject<number | null>) {
  if (ref.current != null) window.clearTimeout(ref.current)
  ref.current = null
}

interface SplitPreviewProps {
  side: 'left' | 'right'
  /** Columns the tab has now — the schematic draws them beside the new one. */
  columns: number
  /** First-run hint: the other edge works too. */
  nudge?: boolean
  disabled?: boolean
  /** Presence: mounting slides in from the edge; `closing` fades out before
      the zone unmounts the card. */
  closing?: boolean
  buttonRef?: RefObject<HTMLButtonElement | null>
  onAdd: () => void
  onKeyDown?: (event: KeyboardEvent<HTMLButtonElement>) => void
  onBlur?: (event: FocusEvent<HTMLButtonElement>) => void
}

/**
 * The card a workspace edge reveals: a floating, pane-shaped surface that
 * previews the split — a schematic of the columns the tab will have, with
 * the new one drawn in ink on the side it will appear — under the action's
 * name and its key. It is the desktop counterpart of the phone's "swipe to
 * split" screen: same `bg-surface` tile, same words.
 */
export function SplitPreview({
  side,
  columns,
  nudge = false,
  disabled = false,
  closing = false,
  buttonRef,
  onAdd,
  onKeyDown,
  onBlur,
}: SplitPreviewProps) {
  const splitLabel = `Split ${side}`
  const action = side === 'left' ? 'panel.splitLeft' : 'panel.split'
  const binding = bindingsFor(action)[0]
  const existing = Math.min(Math.max(Math.trunc(columns), 1), SCHEMATIC_MAX)
  const newColumn = side === 'left' ? 0 : existing
  return (
    <button
      ref={buttonRef}
      type="button"
      disabled={disabled}
      aria-label={splitLabel}
      title={hoverTitle(splitLabel, action)}
      onClick={onAdd}
      onKeyDown={onKeyDown}
      onBlur={onBlur}
      className={cn(
        'group/preview absolute top-0 bottom-1.5 flex w-[7.25rem] flex-col items-center justify-center gap-2.5 px-3 text-center',
        // Overlay grammar: one surface step up and the floating shadow — no
        // frame. It sits over the neighbouring pane, not beside it.
        'rounded-sm bg-panel-raised font-sans shadow-floating select-none',
        'focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent',
        side === 'left' ? 'left-1.5' : 'right-1.5',
        closing
          ? side === 'left'
            ? 'split-preview-exit-left'
            : 'split-preview-exit-right'
          : side === 'left'
            ? 'split-preview-enter-left'
            : 'split-preview-enter-right',
      )}
    >
      {/* The schematic: today's columns in alpha ink, the new one solid —
          it arrives a beat after the card, the way the real pane will. */}
      <span
        aria-hidden
        className={cn(
          'iii-ui-motion-control flex h-9 items-center gap-[3px] rounded-sm bg-surface px-2.5 transition-colors',
          'group-hover/preview:bg-surface-hover group-active/preview:bg-surface-active',
        )}
      >
        {Array.from({ length: existing + 1 }, (_, index) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: position is the identity
            key={index}
            className={cn(
              'h-5 w-3 shrink-0',
              index === newColumn
                ? cn(
                    'bg-ink',
                    side === 'left'
                      ? 'split-preview-column-left'
                      : 'split-preview-column-right',
                  )
                : 'bg-ink/20',
            )}
          />
        ))}
      </span>
      <span className="text-[12.5px] leading-tight font-semibold text-ink">
        {splitLabel}
      </span>
      {binding ? <KeyCombo binding={binding} /> : null}
      {nudge ? (
        <span className="text-[11px] leading-snug text-ink-faint">
          The {side === 'left' ? 'right' : 'left'} edge works too.
        </span>
      ) : null}
    </button>
  )
}

interface EdgeAddZoneProps {
  side: 'left' | 'right'
  onAdd: () => void
  /** Columns the tab has now — the split preview draws them beside the new
      one. */
  columns?: number
  /** Suspend interaction (and dismiss an open preview) during panel motion. */
  disabled?: boolean
  /** First-run discoverability: until the user grows a split once (either
      side) the edge keeps a framed `+` sliver that shakes now and then, and
      the preview mentions the other edge. Afterwards the edge is bare — the
      preview alone answers a dwell, a tap or a keyboard activation. */
  nudge?: boolean
}

/**
 * The far-left / far-right sliver of the workspace. A mouse resting on it
 * reveals the SPLIT PREVIEW — a floating, pane-shaped card sliding in from
 * the edge (a tap or a keyboard activation reveals immediately; touch has
 * no hover to dwell on); activating the preview makes the pane real.
 * Render while the tab is under the workspace's defensive safety ceiling.
 */
export function EdgeAddZone({
  side,
  onAdd,
  columns = 1,
  disabled = false,
  nudge = false,
}: EdgeAddZoneProps) {
  const splitLabel = `Split ${side}`
  const [phase, setPhase] = useState<PreviewPhase>('idle')
  const reducedMotion = useMediaQuery(REDUCED_MOTION_QUERY)
  const dwellRef = useRef<number | null>(null)
  const hideRef = useRef<number | null>(null)
  const exitRef = useRef<number | null>(null)
  const previewRef = useRef<HTMLButtonElement>(null)
  const targetRef = useRef<HTMLButtonElement>(null)
  // Where focus lands once the next phase renders: the preview after a
  // keyboard activation (a dwell or a click never moves focus away from the
  // page), the bare target after Escape.
  const focusRef = useRef<'preview' | 'target' | null>(null)
  const revealed = phase !== 'idle'

  useEffect(
    () => () => {
      clearTimer(dwellRef)
      clearTimer(hideRef)
      clearTimer(exitRef)
    },
    [],
  )
  useEffect(() => {
    if (!disabled) return
    clearTimer(dwellRef)
    clearTimer(hideRef)
    clearTimer(exitRef)
    focusRef.current = null
    setPhase('idle')
  }, [disabled])
  useEffect(() => {
    const wanted = focusRef.current
    if (wanted === 'preview' && phase === 'open') {
      focusRef.current = null
      previewRef.current?.focus()
    } else if (wanted === 'target' && phase === 'idle') {
      focusRef.current = null
      targetRef.current?.focus()
    }
  }, [phase])

  const clearTimers = () => {
    clearTimer(dwellRef)
    clearTimer(hideRef)
    clearTimer(exitRef)
  }
  const reveal = (focusPreview: boolean) => {
    clearTimers()
    focusRef.current = focusPreview ? 'preview' : null
    setPhase('open')
  }
  const hide = (focusTarget = false) => {
    clearTimers()
    focusRef.current = focusTarget ? 'target' : null
    if (reducedMotion) {
      setPhase('idle')
      return
    }
    setPhase((current) => (current === 'idle' ? current : 'closing'))
    exitRef.current = window.setTimeout(() => {
      exitRef.current = null
      setPhase('idle')
    }, EDGE_EXIT_MS)
  }

  return (
    <div
      className={cn(
        'absolute inset-y-0 z-20 hidden pb-1.5 sm:flex',
        disabled && 'pointer-events-none',
        side === 'left' ? 'left-0' : 'right-0',
        revealed ? 'w-32' : 'w-3 sm:w-4',
      )}
      onPointerEnter={(e) => {
        if (disabled) return
        clearTimer(hideRef)
        if (phase === 'closing') {
          reveal(false)
          return
        }
        if (revealed || dwellRef.current != null) return
        if (e.pointerType === 'mouse') {
          dwellRef.current = window.setTimeout(() => {
            dwellRef.current = null
            reveal(false)
          }, EDGE_REVEAL_MS)
        }
      }}
      onPointerLeave={() => {
        if (disabled) return
        clearTimer(dwellRef)
        if (phase !== 'open') return
        // A preview the keyboard opened stays until Escape or blur; a pointer
        // merely passing through must not take it away.
        if (document.activeElement === previewRef.current) return
        hideRef.current = window.setTimeout(() => {
          hideRef.current = null
          hide()
        }, EDGE_HIDE_MS)
      }}
    >
      {revealed ? (
        <SplitPreview
          side={side}
          columns={columns}
          nudge={nudge}
          disabled={disabled}
          closing={phase === 'closing'}
          buttonRef={previewRef}
          onAdd={() => {
            clearTimers()
            focusRef.current = null
            setPhase('idle')
            onAdd()
          }}
          onKeyDown={(event) => {
            if (event.key !== 'Escape') return
            event.preventDefault()
            event.stopPropagation()
            hide(true)
          }}
          onBlur={() => hide()}
        />
      ) : (
        <>
          <button
            ref={targetRef}
            type="button"
            disabled={disabled}
            aria-label={`show ${splitLabel} control`}
            // Touch (and impatient mice): first tap reveals, second adds. A
            // keyboard activation (detail 0) also hands the preview focus.
            onClick={(event) => reveal(event.detail === 0)}
            className={cn(
              'peer h-full w-full cursor-default bg-transparent',
              // Before discovery the sliver below carries the focus
              // treatment; a bare edge draws its own.
              nudge
                ? 'focus-visible:outline-none'
                : 'focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent',
            )}
          />
          {nudge ? (
            // A persistent sliver of the would-be panel, shaking now and then
            // until the first split; afterwards the edge is bare.
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
                  'edge-nudge',
                  side === 'right' && '[animation-delay:-5s]',
                )}
              >
                <Plus className="size-4 shrink-0" />
              </span>
            </span>
          ) : null}
        </>
      )}
    </div>
  )
}
