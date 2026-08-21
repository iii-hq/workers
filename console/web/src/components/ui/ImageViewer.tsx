import * as DialogPrimitive from '@radix-ui/react-dialog'
import { ImageOff, Maximize, X, ZoomIn, ZoomOut } from 'lucide-react'
import * as React from 'react'
import { PortalScope } from '@/lib/ui-scope'
import { cn } from '@/lib/utils'
import { IconButton } from './IconButton'
import {
  centred,
  clampOffset,
  clampScale,
  dataUrlBytes,
  fitScale,
  MAX_PREVIEW_BYTES,
  type PointerPoint,
  panBy,
  pinchGeometry,
  type Size,
  steppedScale,
  type ViewTransform,
  zoomAbout,
  zoomPercent,
} from './image-viewer-math'

export interface ImageViewerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Image source; `null`/`undefined` renders the unavailable state. */
  src?: string | null
  /** Accessible description of the picture. */
  alt: string
  /** Caption: the attachment name or the file's relative path, never a host path. */
  title?: string
  /** Secondary caption, e.g. a byte size or MIME type. */
  description?: string
  /** Why the source is unavailable, when it is. */
  unavailableReason?: string
  /** Extra controls rendered in the toolbar before Close. */
  actions?: React.ReactNode
}

type Phase =
  | { kind: 'loading' }
  | { kind: 'ready'; image: Size }
  | { kind: 'error'; message: string }
  | { kind: 'unavailable'; message: string }

const KEY_PAN_PX = 48

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/** The shared full-screen image viewer; see DESIGN.md for the interaction contract. */
export function ImageViewer({
  open,
  onOpenChange,
  src,
  alt,
  title,
  description,
  unavailableReason,
  actions,
}: ImageViewerProps) {
  // Radix returns focus to a DialogTrigger; a controlled viewer has none.
  const openerRef = React.useRef<HTMLElement | null>(null)
  React.useLayoutEffect(() => {
    if (open) {
      const active = document.activeElement
      openerRef.current = active instanceof HTMLElement ? active : null
    }
  }, [open])
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <PortalScope>
          <DialogPrimitive.Overlay className="iii-ui-motion-overlay fixed inset-0 z-50 bg-black/85" />
          <DialogPrimitive.Content
            aria-label={title ? `Image: ${title}` : 'Image viewer'}
            className="iii-ui-motion-panel fixed inset-0 z-50 flex flex-col font-sans text-ink focus-visible:outline-none"
            onOpenAutoFocus={(event) => {
              event.preventDefault()
              const content = event.currentTarget as HTMLElement | null
              content
                ?.querySelector<HTMLElement>('[data-image-viewer-root]')
                ?.focus()
            }}
            onCloseAutoFocus={(event) => {
              event.preventDefault()
              const opener = openerRef.current
              if (opener?.isConnected) opener.focus()
            }}
          >
            <DialogPrimitive.Title className="sr-only">
              {title ?? 'Image'}
            </DialogPrimitive.Title>
            <DialogPrimitive.Description className="sr-only">
              {alt}
            </DialogPrimitive.Description>
            {open ? (
              <Stage
                src={src ?? null}
                alt={alt}
                title={title}
                description={description}
                unavailableReason={unavailableReason}
                actions={actions}
                onClose={() => onOpenChange(false)}
              />
            ) : null}
          </DialogPrimitive.Content>
        </PortalScope>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  )
}

interface StageProps {
  src: string | null
  alt: string
  title?: string
  description?: string
  unavailableReason?: string
  actions?: React.ReactNode
  onClose: () => void
}

function Stage({
  src,
  alt,
  title,
  description,
  unavailableReason,
  actions,
  onClose,
}: StageProps) {
  const stageRef = React.useRef<HTMLDivElement>(null)
  const imageRef = React.useRef<HTMLImageElement>(null)
  const viewRef = React.useRef<ViewTransform>(centred(1))
  const frameRef = React.useRef<number | null>(null)
  const pointersRef = React.useRef<Map<number, PointerPoint>>(new Map())
  const gestureRef = React.useRef<{
    kind: 'pan' | 'pinch'
    startView: ViewTransform
    startDistance: number
    startMid: { x: number; y: number }
    last: { x: number; y: number }
    moved: boolean
  } | null>(null)
  const [phase, setPhase] = React.useState<Phase>(() =>
    initialPhase(src, unavailableReason),
  )
  const [stageSize, setStageSize] = React.useState<Size>({
    width: 0,
    height: 0,
  })
  const [percent, setPercent] = React.useState('')
  const [scale, setScale] = React.useState(1)
  const [animated, setAnimated] = React.useState(false)
  const [dragging, setDragging] = React.useState(false)

  const image = phase.kind === 'ready' ? phase.image : null
  const fit = image ? fitScale(image, stageSize) : 1

  React.useEffect(() => {
    setPhase(initialPhase(src, unavailableReason))
  }, [src, unavailableReason])

  React.useLayoutEffect(() => {
    const stage = stageRef.current
    if (!stage) return
    const measure = () =>
      setStageSize({ width: stage.clientWidth, height: stage.clientHeight })
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(stage)
    return () => observer.disconnect()
  }, [])

  const paint = React.useCallback(() => {
    frameRef.current = null
    const el = imageRef.current
    if (!el) return
    const view = viewRef.current
    el.style.transform = `translate(${view.x}px, ${view.y}px) scale(${view.scale})`
    setPercent(zoomPercent(view.scale))
    setScale(view.scale)
  }, [])

  const schedulePaint = React.useCallback(() => {
    if (frameRef.current !== null) return
    frameRef.current = requestAnimationFrame(paint)
  }, [paint])

  React.useEffect(
    () => () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current)
    },
    [],
  )

  const apply = React.useCallback(
    (view: ViewTransform, animate = false) => {
      viewRef.current = view
      setAnimated(animate && !prefersReducedMotion())
      schedulePaint()
    },
    [schedulePaint],
  )

  // A loaded image, or a resized stage, starts from fit.
  const fitKey = image ? `${image.width}x${image.height}` : ''
  const lastFitKeyRef = React.useRef('')
  React.useLayoutEffect(() => {
    if (!image || stageSize.width === 0) return
    if (lastFitKeyRef.current === fitKey) {
      apply(clampOffset(viewRef.current, image, stageSize))
      return
    }
    lastFitKeyRef.current = fitKey
    apply(centred(fit))
  }, [image, stageSize, fit, fitKey, apply])

  const stagePoint = React.useCallback((clientX: number, clientY: number) => {
    const rect = stageRef.current?.getBoundingClientRect()
    if (!rect) return { x: 0, y: 0 }
    return {
      x: clientX - rect.left - rect.width / 2,
      y: clientY - rect.top - rect.height / 2,
    }
  }, [])

  const zoomTo = React.useCallback(
    (scale: number, focus = { x: 0, y: 0 }, animate = false) => {
      if (!image) return
      apply(
        zoomAbout(viewRef.current, scale, focus, image, stageSize, fit),
        animate,
      )
    },
    [apply, image, stageSize, fit],
  )

  const zoomStep = React.useCallback(
    (direction: 1 | -1) => {
      zoomTo(steppedScale(viewRef.current.scale, direction))
    },
    [zoomTo],
  )

  const toggleFitActual = React.useCallback(
    (focus = { x: 0, y: 0 }) => {
      const atFit = Math.abs(viewRef.current.scale - fit) < 1e-3
      zoomTo(atFit && fit < 1 ? 1 : fit, focus, true)
    },
    [fit, zoomTo],
  )

  // Native non-passive listener: React's synthetic wheel cannot preventDefault.
  React.useEffect(() => {
    const stage = stageRef.current
    if (!stage) return
    const onWheel = (event: WheelEvent) => {
      event.preventDefault()
      if (!image) return
      const factor = Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.002))
      zoomTo(
        viewRef.current.scale * factor,
        stagePoint(event.clientX, event.clientY),
      )
    }
    stage.addEventListener('wheel', onWheel, { passive: false })
    return () => stage.removeEventListener('wheel', onWheel)
  }, [image, stagePoint, zoomTo])

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!image || event.button !== 0) return
    event.currentTarget.setPointerCapture(event.pointerId)
    pointersRef.current.set(event.pointerId, {
      id: event.pointerId,
      x: event.clientX,
      y: event.clientY,
    })
    const points = [...pointersRef.current.values()]
    const pinch = pinchGeometry(points)
    if (pinch) {
      gestureRef.current = {
        kind: 'pinch',
        startView: viewRef.current,
        startDistance: pinch.distance,
        startMid: stagePoint(pinch.midpoint.x, pinch.midpoint.y),
        last: pinch.midpoint,
        moved: true,
      }
      return
    }
    gestureRef.current = {
      kind: 'pan',
      startView: viewRef.current,
      startDistance: 0,
      startMid: { x: 0, y: 0 },
      last: { x: event.clientX, y: event.clientY },
      moved: false,
    }
  }

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const gesture = gestureRef.current
    if (!gesture || !image) return
    const point = pointersRef.current.get(event.pointerId)
    if (!point) return
    point.x = event.clientX
    point.y = event.clientY
    if (gesture.kind === 'pinch') {
      const pinch = pinchGeometry([...pointersRef.current.values()])
      if (!pinch || gesture.startDistance === 0) return
      const scale = clampScale(
        gesture.startView.scale * (pinch.distance / gesture.startDistance),
        fit,
      )
      const mid = stagePoint(pinch.midpoint.x, pinch.midpoint.y)
      const zoomed = zoomAbout(
        gesture.startView,
        scale,
        gesture.startMid,
        image,
        stageSize,
        fit,
      )
      apply(
        panBy(
          zoomed,
          mid.x - gesture.startMid.x,
          mid.y - gesture.startMid.y,
          image,
          stageSize,
        ),
      )
      return
    }
    const dx = event.clientX - gesture.last.x
    const dy = event.clientY - gesture.last.y
    gesture.last = { x: event.clientX, y: event.clientY }
    if (!gesture.moved && Math.hypot(dx, dy) < 3) return
    if (!gesture.moved) {
      gesture.moved = true
      setDragging(true)
    }
    apply(panBy(viewRef.current, dx, dy, image, stageSize))
  }

  const onPointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    pointersRef.current.delete(event.pointerId)
    try {
      event.currentTarget.releasePointerCapture(event.pointerId)
    } catch {
      // Already released.
    }
    const gesture = gestureRef.current
    if (pointersRef.current.size === 0) {
      gestureRef.current = null
      setDragging(false)
      return
    }
    if (gesture?.kind === 'pinch' && pointersRef.current.size === 1) {
      const [remaining] = pointersRef.current.values()
      gestureRef.current = {
        kind: 'pan',
        startView: viewRef.current,
        startDistance: 0,
        startMid: { x: 0, y: 0 },
        last: { x: remaining.x, y: remaining.y },
        moved: true,
      }
    }
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!image) return
    const target = event.target as HTMLElement
    if (
      target.tagName === 'BUTTON' &&
      (event.key === ' ' || event.key === 'Enter')
    ) {
      return
    }
    switch (event.key) {
      case '+':
      case '=':
        zoomStep(1)
        break
      case '-':
      case '_':
        zoomStep(-1)
        break
      case '0':
        zoomTo(fit)
        break
      case '1':
        zoomTo(1)
        break
      case 'ArrowLeft':
        apply(panBy(viewRef.current, KEY_PAN_PX, 0, image, stageSize))
        break
      case 'ArrowRight':
        apply(panBy(viewRef.current, -KEY_PAN_PX, 0, image, stageSize))
        break
      case 'ArrowUp':
        apply(panBy(viewRef.current, 0, KEY_PAN_PX, image, stageSize))
        break
      case 'ArrowDown':
        apply(panBy(viewRef.current, 0, -KEY_PAN_PX, image, stageSize))
        break
      default:
        return
    }
    event.preventDefault()
  }

  const onLoad = (event: React.SyntheticEvent<HTMLImageElement>) => {
    const el = event.currentTarget
    const size = { width: el.naturalWidth, height: el.naturalHeight }
    if (size.width === 0 || size.height === 0) {
      setPhase({ kind: 'error', message: 'The image could not be decoded.' })
      return
    }
    const decode = el.decode ? el.decode() : Promise.resolve()
    decode
      .then(() => setPhase({ kind: 'ready', image: size }))
      .catch(() =>
        setPhase({ kind: 'error', message: 'The image could not be decoded.' }),
      )
  }

  const zoomable = image !== null
  const atFit = Math.abs(scale - fit) < 1e-3
  const atActual = Math.abs(scale - 1) < 1e-3
  const pannable =
    zoomable &&
    (image.width * viewRef.current.scale > stageSize.width ||
      image.height * viewRef.current.scale > stageSize.height)

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: focusable root (tabIndex -1) that owns the viewer keys.
    <div
      data-image-viewer-root
      tabIndex={-1}
      className="relative flex min-h-0 flex-1 flex-col outline-none"
      onKeyDown={onKeyDown}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 z-10 flex flex-col items-end gap-2 p-3 sm:flex-row sm:items-start sm:justify-between sm:gap-3 sm:p-4">
        <div className="pointer-events-auto order-2 max-w-full min-w-0 rounded-md bg-panel-raised/95 px-3 py-2 shadow-floating sm:order-1 sm:max-w-[40%]">
          <div className="truncate font-sans text-[13px] font-medium text-ink">
            {title ?? 'Image'}
          </div>
          {description || image ? (
            <div className="truncate font-mono text-[11px] text-ink-faint tabular-nums">
              {[image ? `${image.width} × ${image.height}` : null, description]
                .filter(Boolean)
                .join(' · ')}
            </div>
          ) : null}
        </div>
        <div className="pointer-events-auto order-1 flex shrink-0 items-center gap-0.5 rounded-md bg-panel-raised/95 p-1 shadow-floating sm:order-2">
          <IconButton
            label="Zoom out"
            tooltip="Zoom out (-)"
            disabled={!zoomable}
            onClick={() => zoomStep(-1)}
            className="size-11 sm:size-[30px]"
          >
            <ZoomOut aria-hidden />
          </IconButton>
          <output
            aria-live="polite"
            aria-label="Zoom"
            className="min-w-[3.5rem] text-center font-mono text-[12px] text-ink tabular-nums"
          >
            {zoomable ? percent : '–'}
          </output>
          <IconButton
            label="Zoom in"
            tooltip="Zoom in (+)"
            disabled={!zoomable}
            onClick={() => zoomStep(1)}
            className="size-11 sm:size-[30px]"
          >
            <ZoomIn aria-hidden />
          </IconButton>
          <IconButton
            label="Fit to screen"
            tooltip="Fit to screen (0)"
            disabled={!zoomable}
            aria-pressed={zoomable && atFit}
            onClick={() => zoomTo(fit, { x: 0, y: 0 }, true)}
            className={cn(
              'size-11 sm:size-[30px]',
              zoomable && atFit && 'bg-surface-selected text-ink',
            )}
          >
            <Maximize aria-hidden />
          </IconButton>
          <IconButton
            label="Actual size"
            tooltip="Actual size (1)"
            disabled={!zoomable}
            aria-pressed={zoomable && atActual}
            onClick={() => zoomTo(1, { x: 0, y: 0 }, true)}
            className={cn(
              'size-11 sm:size-[30px] font-mono text-[11px] font-semibold tracking-tight',
              zoomable && atActual && 'bg-surface-selected text-ink',
            )}
          >
            <span aria-hidden>1:1</span>
          </IconButton>
          {actions}
          <IconButton
            label="Close"
            tooltip="Close (Esc)"
            onClick={onClose}
            className="size-11 sm:size-[30px]"
          >
            <X aria-hidden />
          </IconButton>
        </div>
      </div>

      {/* biome-ignore lint/a11y/noStaticElementInteractions: pointer surface; the keyboard lives on the dialog content above. */}
      <div
        ref={stageRef}
        data-image-viewer-stage
        className={cn(
          'relative flex min-h-0 flex-1 select-none items-center justify-center overflow-hidden',
          zoomable && (pannable || dragging)
            ? dragging
              ? 'cursor-grabbing'
              : 'cursor-grab'
            : 'cursor-default',
        )}
        style={{ touchAction: 'none' }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onDoubleClick={(event) => {
          if (!image) return
          toggleFitActual(stagePoint(event.clientX, event.clientY))
        }}
      >
        {src && phase.kind !== 'unavailable' ? (
          <img
            ref={imageRef}
            src={src}
            alt={alt}
            draggable={false}
            onLoad={onLoad}
            onError={() =>
              setPhase({
                kind: 'error',
                message: 'The image could not be loaded.',
              })
            }
            className={cn(
              'max-w-none origin-center select-none',
              phase.kind === 'ready' ? 'opacity-100' : 'opacity-0',
              animated &&
                'transition-transform duration-(--motion-duration-control) ease-(--motion-ease-standard)',
            )}
            style={{ willChange: 'transform' }}
            onTransitionEnd={() => setAnimated(false)}
          />
        ) : null}
        {phase.kind === 'loading' ? (
          <div
            role="status"
            className="absolute rounded-md bg-panel-raised/95 px-3 py-2 font-sans text-[12px] text-ink-faint shadow-floating"
          >
            Loading image…
          </div>
        ) : null}
        {phase.kind === 'error' || phase.kind === 'unavailable' ? (
          <div
            role="status"
            className="absolute flex max-w-sm flex-col items-center gap-2 rounded-md bg-panel-raised px-5 py-4 text-center shadow-floating"
          >
            <ImageOff aria-hidden className="size-5 text-ink-faint" />
            <div className="font-sans text-[13px] font-medium text-ink">
              {phase.kind === 'error'
                ? 'Cannot show this image'
                : 'Image not available'}
            </div>
            <div className="font-sans text-[12px] text-ink-faint">
              {phase.message}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  )
}

function initialPhase(src: string | null, unavailableReason?: string): Phase {
  if (!src) {
    return {
      kind: 'unavailable',
      message:
        unavailableReason ?? 'The image bytes were not kept with this item.',
    }
  }
  const bytes = dataUrlBytes(src)
  if (bytes !== null && bytes > MAX_PREVIEW_BYTES) {
    return {
      kind: 'unavailable',
      message: `This image is ${(bytes / (1024 * 1024)).toFixed(0)} MB, above the ${MAX_PREVIEW_BYTES / (1024 * 1024)} MB preview limit.`,
    }
  }
  return { kind: 'loading' }
}

export interface ImageThumbnailButtonProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, 'title'> {
  /** What opens: the viewer's caption. */
  title: string
}

/** The opener: a button around a thumbnail, with the zoom cursor and a name. */
export const ImageThumbnailButton = React.forwardRef<
  HTMLButtonElement,
  ImageThumbnailButtonProps
>(({ title, className, children, ...props }, ref) => (
  <button
    ref={ref}
    type="button"
    aria-label={`View ${title}`}
    title={`View ${title}`}
    className={cn(
      'inline-flex cursor-zoom-in items-center justify-center rounded-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus',
      className,
    )}
    {...props}
  >
    {children}
  </button>
))
ImageThumbnailButton.displayName = 'ImageThumbnailButton'

/** Open/close state plus the opener wiring for one image. */
export function useImageViewer(): {
  open: boolean
  setOpen: (open: boolean) => void
  show: () => void
} {
  const [open, setOpen] = React.useState(false)
  const show = React.useCallback(() => setOpen(true), [])
  return { open, setOpen, show }
}
