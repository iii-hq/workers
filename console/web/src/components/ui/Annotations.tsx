/**
 * Numbered pins over a picture, each with a note written beside it.
 *
 * `AnnotationLayer` wraps the picture element and paints the pins on the
 * painted part of an `object-fit: contain` image; clicks in the layer add a
 * pin while `active`, a pin is a real button (focus, Delete, arrows nudge),
 * and with `onNote` the selected pin opens a callout that edits its note in
 * place. `AnnotationList` is the same notes as rows, for pages that want a
 * list. State stays with the caller; both are pure views of one list.
 */

import { Trash2 } from 'lucide-react'
import {
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { IconButton } from '@/components/ui/IconButton'
import { Input } from '@/components/ui/Input'
import {
  type Annotation,
  type AnnotationKind,
  annotationKind,
  containedImageBox,
} from '@/lib/annotations'
import { cn } from '@/lib/utils'

export interface AnnotationLayerProps {
  annotations: readonly Annotation[]
  /** The picture's pixel size, for the contain box. */
  image: { width: number; height: number }
  /** Clicks add pins; off, the picture behaves as before. */
  active: boolean
  selectedId?: string | null
  onAdd?: (x: number, y: number) => void
  onSelect?: (id: string | null) => void
  onMove?: (id: string, x: number, y: number) => void
  onRemove?: (id: string) => void
  /** With it, the selected pin opens a callout that edits its note in place. */
  onNote?: (id: string, note: string) => void
  /** The active tool. `pin` (default) drops a pin on click; `rect` / `arrow`
   * draw a shape on drag. */
  tool?: AnnotationKind
  /** Colour for a newly drawn shape. */
  drawColor?: string
  /** Begin a shape at a point (rect corner / arrow tail). */
  onAddShape?: (kind: 'rect' | 'arrow', x: number, y: number) => void
  /** Update the shape being drawn (its far corner / arrow head). */
  onResizeShape?: (x2: number, y2: number) => void
  /** Finish the shape being drawn; the caller drops a too-small one. */
  onEndShape?: () => void
  className?: string
  /** The picture element, rendered by the caller. */
  children: ReactNode
}

const NUDGE = 0.005
const CALLOUT_WIDTH = 264
const CALLOUT_GAP = 22

export function AnnotationLayer({
  annotations,
  image,
  active,
  selectedId = null,
  onAdd,
  onSelect,
  onMove,
  onRemove,
  onNote,
  tool = 'pin',
  onAddShape,
  onResizeShape,
  onEndShape,
  className,
  children,
}: AnnotationLayerProps) {
  const rootRef = useRef<HTMLDivElement>(null)
  const [box, setBox] = useState({ left: 0, top: 0, width: 0, height: 0 })
  const [rootWidth, setRootWidth] = useState(0)
  const imageWidth = image.width
  const imageHeight = image.height
  const measure = useCallback(() => {
    const root = rootRef.current
    if (!root) return
    setRootWidth(root.clientWidth)
    const next = containedImageBox(
      { width: root.clientWidth, height: root.clientHeight },
      { width: imageWidth, height: imageHeight },
    )
    // Callers pass `image` as a fresh object each render; comparing the
    // computed box keeps a same-size remeasure from re-rendering.
    setBox((current) =>
      current.left === next.left &&
      current.top === next.top &&
      current.width === next.width &&
      current.height === next.height
        ? current
        : next,
    )
  }, [imageWidth, imageHeight])
  useLayoutEffect(measure, [measure])
  useEffect(() => {
    const root = rootRef.current
    if (!root || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(root)
    return () => observer.disconnect()
  }, [measure])

  const fractionAt = (event: PointerEvent<HTMLElement>) => {
    const rect = rootRef.current?.getBoundingClientRect()
    if (!rect || box.width === 0 || box.height === 0) return null
    return {
      x: (event.clientX - rect.left - box.left) / box.width,
      y: (event.clientY - rect.top - box.top) / box.height,
    }
  }

  const pinLeft = (pin: Annotation) => box.left + pin.x * box.width
  const pinTop = (pin: Annotation) => box.top + pin.y * box.height

  const dragRef = useRef<{ id: string; moved: boolean } | null>(null)
  const drawingRef = useRef(false)
  const endDrawing = (event: PointerEvent<HTMLElement>) => {
    if (!drawingRef.current) return
    drawingRef.current = false
    try {
      rootRef.current?.releasePointerCapture(event.pointerId)
    } catch {
      // never captured; nothing to release
    }
    onEndShape?.()
  }

  // The selected pin's note takes the caret: writing it is the point of
  // selecting a pin. Runs after the pointer handlers, so it wins focus.
  const noteRef = useRef<HTMLInputElement>(null)
  const canNote = onNote !== undefined
  useEffect(() => {
    if (selectedId && canNote) noteRef.current?.focus()
  }, [selectedId, canNote])

  const selectedIndex = annotations.findIndex((a) => a.id === selectedId)
  const selected = selectedIndex >= 0 ? annotations[selectedIndex] : null
  const calloutLeft =
    selected && box.width > 0 ? pinLeft(selected) + CALLOUT_GAP : 0
  const calloutFlips = calloutLeft + CALLOUT_WIDTH > rootWidth

  return (
    <div
      ref={rootRef}
      data-annotation-layer={active ? 'active' : 'idle'}
      className={cn('relative', active && 'cursor-crosshair', className)}
      onPointerDown={(event) => {
        if (!active || event.button !== 0) return
        if (
          (event.target as HTMLElement).closest(
            '[data-annotation-pin], [data-annotation-shape], [data-annotation-callout]',
          )
        )
          return
        const at = fractionAt(event)
        if (!at || at.x < 0 || at.x > 1 || at.y < 0 || at.y > 1) return
        event.preventDefault()
        if (tool === 'pin') {
          onAdd?.(at.x, at.y)
          return
        }
        if (!onAddShape) return
        drawingRef.current = true
        try {
          rootRef.current?.setPointerCapture(event.pointerId)
        } catch {
          // a synthetic or already-released pointer cannot be captured;
          // drawing still works, the capture is just a nicety for fast drags
        }
        onAddShape(tool, at.x, at.y)
      }}
      onPointerMove={(event) => {
        if (!drawingRef.current) return
        const at = fractionAt(event)
        if (!at) return
        onResizeShape?.(
          Math.min(1, Math.max(0, at.x)),
          Math.min(1, Math.max(0, at.y)),
        )
      }}
      onPointerUp={endDrawing}
      onPointerCancel={endDrawing}
    >
      {children}
      {annotations.map((annotation) => {
        const kind = annotationKind(annotation)
        if (kind === 'pin') return null
        const sel = annotation.id === selectedId
        const color = annotation.color ?? 'var(--color-accent)'
        const x1 = Math.min(annotation.x, annotation.x2 ?? annotation.x)
        const y1 = Math.min(annotation.y, annotation.y2 ?? annotation.y)
        const x2 = Math.max(annotation.x, annotation.x2 ?? annotation.x)
        const y2 = Math.max(annotation.y, annotation.y2 ?? annotation.y)
        if (kind === 'rect') {
          return (
            <button
              key={annotation.id}
              type="button"
              data-annotation-shape={annotation.id}
              aria-label={`rectangle${annotation.note ? `: ${annotation.note}` : ''}`}
              aria-pressed={sel}
              onPointerDown={(event) => {
                event.stopPropagation()
                onSelect?.(annotation.id)
              }}
              onKeyDown={(event) => {
                if (event.key === 'Delete' || event.key === 'Backspace') {
                  event.preventDefault()
                  onRemove?.(annotation.id)
                }
              }}
              style={{
                left: box.left + x1 * box.width,
                top: box.top + y1 * box.height,
                width: (x2 - x1) * box.width,
                height: (y2 - y1) * box.height,
                borderColor: color,
              }}
              className={cn(
                'absolute z-10 border-2 bg-transparent',
                sel && 'ring-2 ring-white/70 ring-offset-1',
              )}
            />
          )
        }
        return (
          <svg
            key={annotation.id}
            data-annotation-shape={annotation.id}
            aria-label={`arrow${annotation.note ? `: ${annotation.note}` : ''}`}
            className="pointer-events-none absolute inset-0 z-10 size-full overflow-visible"
          >
            <title>{annotation.note || 'arrow'}</title>
            <defs>
              <marker
                id={`arrow-${annotation.id}`}
                viewBox="0 0 10 10"
                refX="8"
                refY="5"
                markerWidth="7"
                markerHeight="7"
                orient="auto-start-reverse"
              >
                <path d="M0 0 L10 5 L0 10 z" fill={color} />
              </marker>
            </defs>
            <line
              x1={box.left + annotation.x * box.width}
              y1={box.top + annotation.y * box.height}
              x2={box.left + (annotation.x2 ?? annotation.x) * box.width}
              y2={box.top + (annotation.y2 ?? annotation.y) * box.height}
              stroke={color}
              strokeWidth={3}
              strokeLinecap="round"
              markerEnd={`url(#arrow-${annotation.id})`}
              className="pointer-events-auto cursor-pointer"
              onPointerDown={(event) => {
                event.stopPropagation()
                onSelect?.(annotation.id)
              }}
            />
          </svg>
        )
      })}
      {annotations.map((annotation, index) => {
        if (annotationKind(annotation) !== 'pin') return null
        const isSelected = annotation.id === selectedId
        return (
          <button
            key={annotation.id}
            type="button"
            data-annotation-pin={annotation.id}
            aria-label={`annotation ${index + 1}${annotation.note ? `: ${annotation.note}` : ''}`}
            aria-pressed={isSelected}
            title={annotation.note || annotation.label}
            style={{
              left: pinLeft(annotation),
              top: pinTop(annotation),
            }}
            className={cn(
              'absolute z-10 flex size-7 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full border-2 border-white bg-accent font-mono text-[12px] font-semibold text-accent-fg shadow-md transition-transform [transition-duration:var(--motion-duration-control)] [transition-timing-function:var(--motion-ease-standard)] hover:scale-110 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent active:scale-95',
              isSelected &&
                'ring-2 ring-white/80 ring-offset-2 ring-offset-accent',
              active ? 'cursor-grab' : 'cursor-pointer',
            )}
            onPointerDown={(event) => {
              event.stopPropagation()
              onSelect?.(annotation.id)
              if (!active || !onMove) return
              dragRef.current = { id: annotation.id, moved: false }
              event.currentTarget.setPointerCapture(event.pointerId)
            }}
            onPointerMove={(event) => {
              const drag = dragRef.current
              if (!drag || drag.id !== annotation.id) return
              const at = fractionAt(event)
              if (!at) return
              drag.moved = true
              onMove?.(annotation.id, at.x, at.y)
            }}
            onPointerUp={(event) => {
              if (dragRef.current?.id === annotation.id) {
                event.currentTarget.releasePointerCapture(event.pointerId)
                dragRef.current = null
              }
            }}
            onPointerCancel={(event) => {
              if (dragRef.current?.id === annotation.id) {
                try {
                  event.currentTarget.releasePointerCapture(event.pointerId)
                } catch {
                  // never captured; nothing to release
                }
                dragRef.current = null
              }
            }}
            onKeyDown={(event) => {
              if (event.key === 'Delete' || event.key === 'Backspace') {
                event.preventDefault()
                onRemove?.(annotation.id)
                return
              }
              const step = event.shiftKey ? NUDGE * 4 : NUDGE
              const delta: Record<string, [number, number]> = {
                ArrowLeft: [-step, 0],
                ArrowRight: [step, 0],
                ArrowUp: [0, -step],
                ArrowDown: [0, step],
              }
              const move = delta[event.key]
              if (!move || !onMove) return
              event.preventDefault()
              onMove(
                annotation.id,
                annotation.x + move[0],
                annotation.y + move[1],
              )
            }}
          >
            {index + 1}
          </button>
        )
      })}
      {selected && onNote ? (
        <fieldset
          data-annotation-callout={selected.id}
          aria-label={`note for annotation ${selectedIndex + 1}`}
          style={{
            width: CALLOUT_WIDTH,
            left: calloutFlips
              ? Math.max(0, calloutLeft - CALLOUT_GAP * 2 - CALLOUT_WIDTH)
              : calloutLeft,
            top: pinTop(selected),
          }}
          className="absolute z-20 flex -translate-y-1/2 flex-col gap-1 rounded-sm border border-edge bg-panel p-2 shadow-md"
          onPointerDown={(event) => event.stopPropagation()}
        >
          <div className="flex items-center gap-1.5">
            <Input
              ref={noteRef}
              value={selected.note}
              onChange={(next) => onNote(selected.id, next)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === 'Escape') {
                  event.preventDefault()
                  event.stopPropagation()
                  onSelect?.(null)
                }
              }}
              placeholder="What about this?"
              aria-label={`note for annotation ${selectedIndex + 1}`}
              preserveCase
              className="h-8 normal-case"
            />
            <IconButton
              label={`remove annotation ${selectedIndex + 1}`}
              onClick={() => onRemove?.(selected.id)}
            >
              <Trash2 aria-hidden className="size-4" />
            </IconButton>
          </div>
          <PinLabel label={selected.label} />
        </fieldset>
      ) : null}
    </div>
  )
}

export interface AnnotationListProps {
  annotations: readonly Annotation[]
  selectedId?: string | null
  onSelect?: (id: string | null) => void
  onNote: (id: string, note: string) => void
  onRemove: (id: string) => void
  /** What an empty list says; the caller knows how pins are added. */
  emptyText?: string
  className?: string
}

export function AnnotationList({
  annotations,
  selectedId = null,
  onSelect,
  onNote,
  onRemove,
  emptyText = 'Click the picture to add a pin.',
  className,
}: AnnotationListProps) {
  const inputs = useRef(new Map<string, HTMLInputElement>())
  const focusNote = useCallback((id: string | undefined) => {
    if (id) inputs.current.get(id)?.focus()
  }, [])
  // A new pin gets the caret: the note is the point of the pin.
  const lastCount = useRef(annotations.length)
  useEffect(() => {
    if (annotations.length > lastCount.current) {
      focusNote(annotations[annotations.length - 1]?.id)
    }
    lastCount.current = annotations.length
  }, [annotations, focusNote])

  if (annotations.length === 0) {
    return (
      <p
        className={cn('px-3 py-4 font-sans text-sm text-ink-faint', className)}
      >
        {emptyText}
      </p>
    )
  }

  const onKeyDown =
    (index: number) => (event: KeyboardEvent<HTMLInputElement>) => {
      if (event.key === 'Enter') {
        event.preventDefault()
        focusNote(annotations[index + 1]?.id)
        return
      }
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        const next = annotations[index + (event.key === 'ArrowDown' ? 1 : -1)]
        if (!next) return
        event.preventDefault()
        focusNote(next.id)
      }
    }

  return (
    <ol className={cn('flex flex-col', className)} aria-label="annotations">
      {annotations.map((annotation, index) => (
        <li
          key={annotation.id}
          data-annotation-row={annotation.id}
          data-selected={annotation.id === selectedId || undefined}
          className="flex items-center gap-2 border-b border-edge px-3 py-1.5 data-[selected]:bg-surface-selected"
        >
          <span
            aria-hidden
            className="flex size-6 shrink-0 items-center justify-center rounded-full bg-accent font-mono text-[11px] font-semibold text-accent-fg"
          >
            {index + 1}
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <Input
              ref={(node) => {
                if (node) inputs.current.set(annotation.id, node)
                else inputs.current.delete(annotation.id)
              }}
              value={annotation.note}
              onChange={(next) => onNote(annotation.id, next)}
              onFocus={() => onSelect?.(annotation.id)}
              onKeyDown={onKeyDown(index)}
              placeholder="Add a note…"
              aria-label={`note for annotation ${index + 1}`}
              preserveCase
              className="h-8 normal-case"
            />
            <PinLabel label={annotation.label} />
          </div>
          <IconButton
            label={`remove annotation ${index + 1}`}
            onClick={() => onRemove(annotation.id)}
          >
            <Trash2 aria-hidden className="size-4" />
          </IconButton>
        </li>
      ))}
    </ol>
  )
}

/** What a pin points at, under its note. Nothing when the page knows nothing. */
function PinLabel({ label }: { label?: string }) {
  if (!label) return null
  return (
    <span
      className="truncate px-1 font-mono text-[11px] text-ink-faint"
      title={label}
    >
      {label}
    </span>
  )
}
