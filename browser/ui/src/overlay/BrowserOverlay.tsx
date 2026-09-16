import type { Host } from '@iii-dev/console-ui'
import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import {
  BROWSER_SESSION_STARTED_TRIGGER,
  BROWSER_SESSION_STOPPED_TRIGGER,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { useBrowserEvent } from '../lib/events'
import { Maximize, X } from '../lib/icons'
import { shouldOpenBrowserSession } from '../lib/session-open'
import { useLiveFrames } from '../page/useLiveFrames'
import {
  type Bounds,
  boundsFor,
  clampPoint,
  DRAG_THRESHOLD,
  decayVelocity,
  isStill,
  overshoot,
  type Point,
  rubberBandPoint,
  type Sample,
  velocityFromSamples,
} from './overlay-motion'
import {
  type Limits,
  overlayLimits,
  pinchDistance,
  pinchWidth,
  settleWidth,
} from './overlay-size'
import {
  bringBrowserOverlayToFront,
  browserOverlaySessions,
  dismissBrowserOverlay,
  forgetBrowserOverlay,
  openBrowserPane,
  showBrowserOverlay,
  subscribeBrowserOverlay,
} from './overlay-store'

/**
 * The live preview: small picture-in-picture thumbnails of the tabs agents
 * opened, fed by the same screencast the browser page uses. The group sits
 * in a corner (top-right on touch layouts, bottom-right with a pointer)
 * until dragged elsewhere — one finger or the mouse moves it, a released
 * swipe coasts and settles inside the viewport, a pinch resizes it. Each
 * card keeps its page's aspect ratio; the width and the spot stick across
 * reloads.
 *
 * Several tabs stack like a deck: the newest in front, the others tilted
 * behind it so their corners show. Tapping the front fans the deck out
 * sideways like a hand of cards (and shows the front card's controls:
 * expand into the browser page, hide); tapping any card in the fan brings
 * it to the front and folds the deck again.
 */

const WIDTH_KEY = 'browser-ui:overlay:width'
const POSITION_KEY = 'browser-ui:overlay:position'
const DEFAULT_WIDTH = 220
/** Matches `--panel-close-dur`: the element unmounts after its exit ran. */
const CLOSE_MS = 350
/** Longest frame a fling integrates over (a background tab's rAF pause). */
const MAX_FRAME_MS = 48
/** Past this far outside the viewport a fling stops and snaps back. */
const MAX_OVERSHOOT = 80
const VELOCITY_SAMPLES = 6
/** How much of a card's width the next one in the fan is offset by. */
const FAN_OVERLAP = 0.6
/**
 * Tallest a card gets, as a share of the pinched width: a tab on a
 * portrait viewport keeps the page's exact aspect ratio but gives up width
 * instead of towering over the landscape ones.
 */
const MAX_HEIGHT_RATIO = 0.7

/** Folded tilt of the card `depth` places behind the front: alternating
 * sides, a little more each step, so every corner peeks out somewhere. */
function foldedTilt(depth: number): number {
  if (depth === 0) return 0
  return (depth % 2 === 1 ? -1 : 1) * (2 + 3 * depth)
}

function readStored<T>(
  key: string,
  parse: (raw: string) => T | null,
): T | null {
  try {
    const raw = window.localStorage.getItem(key)
    return raw === null ? null : parse(raw)
  } catch {
    return null
  }
}

function writeStored(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value)
  } catch {
    /* private mode / quota — persistence is best-effort */
  }
}

function readWidth(): number {
  const stored = readStored(WIDTH_KEY, (raw) => Number(raw))
  return stored !== null && Number.isFinite(stored) && stored > 0
    ? stored
    : DEFAULT_WIDTH
}

function readPosition(): Point {
  const stored = readStored(POSITION_KEY, (raw) => {
    const parsed = JSON.parse(raw) as { x?: unknown; y?: unknown }
    return typeof parsed?.x === 'number' && typeof parsed?.y === 'number'
      ? { x: parsed.x, y: parsed.y }
      : null
  })
  return stored ?? { x: 0, y: 0 }
}

interface Pinch {
  startDistance: number
  startWidth: number
  limits: Limits
}

interface Drag {
  pointerId: number
  start: Point
  origin: Point
  raw: Point
  bounds: Bounds
  samples: Sample[]
  moved: boolean
}

export function BrowserOverlay({ host }: { host: Host }) {
  const sessions = useSyncExternalStore(
    subscribeBrowserOverlay,
    browserOverlaySessions,
  )

  useBrowserEvent({
    host,
    enabled: true,
    triggerType: BROWSER_SESSION_STARTED_TRIGGER,
    fnId: 'iii::browser-ui::overlay-started',
    onEvent: (payload) => {
      const event = payload as { session_id?: unknown; url?: unknown } | null
      if (typeof event?.session_id !== 'string' || !event.session_id) return
      const url = typeof event.url === 'string' ? event.url : undefined
      if (!shouldOpenBrowserSession(url, window.location.origin)) return
      showBrowserOverlay(event.session_id)
    },
  })
  useBrowserEvent({
    host,
    enabled: true,
    triggerType: BROWSER_SESSION_STOPPED_TRIGGER,
    fnId: 'iii::browser-ui::overlay-stopped',
    onEvent: (payload) => {
      const id = (payload as { session_id?: unknown } | null)?.session_id
      if (typeof id === 'string') forgetBrowserOverlay(id)
    },
  })

  // The cards outlive an emptied deck by one exit transition.
  const [shown, setShown] = useState<readonly string[]>([])
  useEffect(() => {
    if (sessions.length > 0) {
      setShown(sessions)
      return
    }
    const timer = window.setTimeout(() => setShown([]), CLOSE_MS)
    return () => window.clearTimeout(timer)
  }, [sessions])
  const front = sessions[sessions.length - 1] ?? null

  // Cards with a picture; the deck reveals once the front one has its own.
  const [ready, setReady] = useState<ReadonlySet<string>>(() => new Set())
  const onReady = useCallback((id: string, hasFrame: boolean) => {
    setReady((current) => {
      if (current.has(id) === hasFrame) return current
      const next = new Set(current)
      if (hasFrame) next.add(id)
      else next.delete(id)
      return next
    })
  }, [])
  const open = front !== null && ready.has(front)

  const [expanded, setExpanded] = useState(false)
  useEffect(() => {
    if (sessions.length === 0) setExpanded(false)
  }, [sessions.length])
  // The fan spreads toward whichever side has more room, and no further
  // than that room allows.
  const [fan, setFan] = useState({ dir: -1, step: 0 })

  const rootRef = useRef<HTMLElement>(null)
  const [width, setWidth] = useState(readWidth)
  const widthRef = useRef(width)
  widthRef.current = width
  const [position, setPosition] = useState(readPosition)
  const positionRef = useRef(position)
  positionRef.current = position
  // Transitions off while a finger or a fling drives the box.
  const [pinching, setPinching] = useState(false)
  const [moving, setMoving] = useState(false)

  const pointers = useRef(new Map<number, Point>())
  const pinch = useRef<Pinch | null>(null)
  const drag = useRef<Drag | null>(null)
  const fling = useRef<number | null>(null)
  // A tap that was part of a drag or a pinch must not act as a click.
  const gesturedRef = useRef(false)

  const settle = useCallback((next: Point) => {
    setMoving(false)
    setPosition(next)
    writeStored(POSITION_KEY, JSON.stringify(next))
  }, [])

  const stopFling = useCallback(() => {
    if (fling.current !== null) {
      window.cancelAnimationFrame(fling.current)
      fling.current = null
    }
  }, [])

  const startFling = useCallback(
    (from: Point, velocity: Point, bounds: Bounds) => {
      let raw = from
      let v = velocity
      let last = performance.now()
      const step = (now: number) => {
        const dt = Math.min(now - last, MAX_FRAME_MS)
        last = now
        raw = { x: raw.x + v.x * dt, y: raw.y + v.y * dt }
        v = decayVelocity(v, dt)
        if (isStill(v) || overshoot(raw, bounds) > MAX_OVERSHOOT) {
          fling.current = null
          settle(clampPoint(raw, bounds))
          return
        }
        setPosition(rubberBandPoint(raw, bounds))
        fling.current = window.requestAnimationFrame(step)
      }
      fling.current = window.requestAnimationFrame(step)
    },
    [settle],
  )

  const currentBounds = useCallback((): Bounds | null => {
    const node = rootRef.current
    if (!node) return null
    return boundsFor(node.getBoundingClientRect(), positionRef.current, {
      width: window.innerWidth,
      height: window.innerHeight,
    })
  }, [])

  // The gesture listens on the window: pointer capture would retarget the
  // click that ends a tap away from the card's own button.
  const onPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLElement>) => {
      if (event.button !== 0 && event.pointerType === 'mouse') return
      const map = pointers.current
      const point = { x: event.clientX, y: event.clientY }
      if (map.size === 0) gesturedRef.current = false
      map.set(event.pointerId, point)
      stopFling()

      if (map.size === 1) {
        const bounds = currentBounds()
        if (!bounds) return
        drag.current = {
          pointerId: event.pointerId,
          start: point,
          origin: positionRef.current,
          raw: positionRef.current,
          bounds,
          samples: [{ ...positionRef.current, t: event.timeStamp }],
          moved: false,
        }
      } else if (map.size === 2) {
        // Two fingers: the drag yields to the pinch where it stands.
        if (drag.current?.moved)
          settle(clampPoint(drag.current.raw, drag.current.bounds))
        drag.current = null
        const [a, b] = [...map.values()]
        pinch.current = {
          startDistance: pinchDistance(a, b) || 1,
          startWidth: widthRef.current,
          limits: overlayLimits(window.innerWidth),
        }
        gesturedRef.current = true
        setPinching(true)
      }

      const onMove = (e: PointerEvent) => {
        if (!map.has(e.pointerId)) return
        map.set(e.pointerId, { x: e.clientX, y: e.clientY })
        const gesture = pinch.current
        if (gesture && map.size >= 2) {
          const [p, q] = [...map.values()]
          setWidth(
            pinchWidth(
              gesture.startWidth,
              pinchDistance(p, q) / gesture.startDistance,
              gesture.limits,
            ),
          )
          return
        }
        const d = drag.current
        if (!d || d.pointerId !== e.pointerId) return
        const dx = e.clientX - d.start.x
        const dy = e.clientY - d.start.y
        if (!d.moved) {
          if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return
          d.moved = true
          gesturedRef.current = true
          setMoving(true)
        }
        d.raw = { x: d.origin.x + dx, y: d.origin.y + dy }
        d.samples.push({ ...d.raw, t: e.timeStamp })
        if (d.samples.length > VELOCITY_SAMPLES) d.samples.shift()
        setPosition(rubberBandPoint(d.raw, d.bounds))
      }
      const onEnd = (e: PointerEvent) => {
        if (!map.delete(e.pointerId)) return
        const gesture = pinch.current
        if (gesture && map.size < 2) {
          pinch.current = null
          setPinching(false)
          setWidth((current) => {
            const settled = settleWidth(current, gesture.limits)
            writeStored(WIDTH_KEY, String(settled))
            return settled
          })
        }
        const d = drag.current
        if (d && d.pointerId === e.pointerId) {
          drag.current = null
          if (d.moved) {
            const velocity = velocityFromSamples(d.samples, e.timeStamp)
            if (isStill(velocity)) settle(clampPoint(d.raw, d.bounds))
            else startFling(d.raw, velocity, d.bounds)
          }
        }
        if (map.size === 0) {
          window.removeEventListener('pointermove', onMove)
          window.removeEventListener('pointerup', onEnd)
          window.removeEventListener('pointercancel', onEnd)
        }
      }
      if (map.size === 1) {
        window.addEventListener('pointermove', onMove)
        window.addEventListener('pointerup', onEnd)
        window.addEventListener('pointercancel', onEnd)
      }
    },
    [currentBounds, settle, startFling, stopFling],
  )

  // A resized viewport (rotation, a window drag) keeps the box in view.
  useEffect(() => {
    const onResize = () => {
      if (drag.current || fling.current !== null) return
      const bounds = currentBounds()
      if (!bounds) return
      const clamped = clampPoint(positionRef.current, bounds)
      if (
        clamped.x !== positionRef.current.x ||
        clamped.y !== positionRef.current.y
      ) {
        settle(clamped)
      }
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [currentBounds, settle])

  useEffect(() => stopFling, [stopFling])

  const cardCount = shown.length
  useEffect(() => {
    if (!expanded) return
    const node = rootRef.current
    if (!node) return
    const rect = node.getBoundingClientRect()
    const roomLeft = rect.left - 8
    const roomRight = window.innerWidth - rect.right - 8
    const dir = roomLeft >= roomRight ? -1 : 1
    const room = Math.max(0, Math.max(roomLeft, roomRight))
    const step =
      cardCount > 1 ? Math.min(width * FAN_OVERLAP, room / (cardCount - 1)) : 0
    setFan({ dir, step })
  }, [expanded, width, cardCount])

  // Folded: the front toggles the fan, a peeking card comes forward. Fanned
  // out: any card comes forward and the deck folds behind it.
  const onCardClick = useCallback((sessionId: string, isFront: boolean) => {
    if (gesturedRef.current) {
      gesturedRef.current = false
      return
    }
    if (!isFront) bringBrowserOverlayToFront(sessionId)
    setExpanded((value) => (isFront ? !value : false))
  }, [])

  if (shown.length === 0) return null

  return (
    <section
      ref={rootRef}
      className={cn(
        'br-ui-pip',
        pinching && 'is-pinching',
        moving && 'is-moving',
      )}
      data-expanded={expanded ? 'true' : 'false'}
      aria-label="Live browser preview"
      style={{
        width,
        transform: `translate(${position.x}px, ${position.y}px)`,
        ['--fan-dir' as string]: fan.dir,
        ['--fan-step' as string]: `${fan.step}px`,
      }}
      onPointerDown={onPointerDown}
    >
      <div
        className="br-ui-pip-deck t-panel-slide"
        data-open={open ? 'true' : 'false'}
      >
        {shown.map((sessionId, index) => {
          const depth = shown.length - 1 - index
          return (
            <Card
              key={sessionId}
              host={host}
              sessionId={sessionId}
              depth={depth}
              live={depth === 0 || expanded}
              expanded={expanded}
              width={width}
              onReady={onReady}
              onClick={onCardClick}
              onExpand={() => {
                setExpanded(false)
                openBrowserPane(host, sessionId)
              }}
              onHide={() => {
                setExpanded(false)
                dismissBrowserOverlay(sessionId)
              }}
            />
          )
        })}
      </div>
    </section>
  )
}

interface CardProps {
  host: Host
  sessionId: string
  /** 0 is the front card; higher sits further back in the deck. */
  depth: number
  /** Stream frames; a folded-away card keeps its last picture instead. */
  live: boolean
  expanded: boolean
  /** The deck's pinched width; a card may be narrower to respect the height cap. */
  width: number
  onReady: (sessionId: string, hasFrame: boolean) => void
  onClick: (sessionId: string, isFront: boolean) => void
  onExpand: () => void
  onHide: () => void
}

function Card({
  host,
  sessionId,
  depth,
  live,
  expanded,
  width,
  onReady,
  onClick,
  onExpand,
  onHide,
}: CardProps) {
  // ponytail: every fanned-out card streams its own screencast; fine for
  // the handful of live tabs max_sessions allows, revisit if the bus
  // suffers with many concurrent tabs.
  const { frame } = useLiveFrames(host, sessionId, live, 0, 0, true)
  const hasFrame = frame !== null
  useEffect(() => {
    onReady(sessionId, hasFrame)
    return () => onReady(sessionId, false)
  }, [onReady, sessionId, hasFrame])

  const isFront = depth === 0
  const aspectRatio = frame ? `${frame.width} / ${frame.height}` : '16 / 10'
  const cardWidth = frame
    ? Math.min(width, (width * MAX_HEIGHT_RATIO * frame.width) / frame.height)
    : width
  return (
    <div
      className="br-ui-pip-card"
      data-front={isFront ? 'true' : 'false'}
      data-ready={hasFrame ? 'true' : 'false'}
      style={{
        ['--depth' as string]: depth,
        ['--tilt' as string]: `${foldedTilt(depth)}deg`,
        width: cardWidth,
        aspectRatio,
        zIndex: 100 - depth,
      }}
    >
      <button
        type="button"
        className="br-ui-pip-surface"
        aria-label={
          isFront
            ? expanded
              ? 'Fold the preview'
              : 'Fan the preview out'
            : 'Bring this tab to the front'
        }
        aria-expanded={isFront ? expanded : undefined}
        onClick={() => onClick(sessionId, isFront)}
      >
        {frame ? <img src={frame.dataUrl} alt="" draggable={false} /> : null}
      </button>
      {isFront ? (
        <div
          className="br-ui-pip-controls"
          data-open={expanded ? 'true' : 'false'}
        >
          <button
            type="button"
            className="br-ui-pip-btn"
            aria-label="Open in browser tab"
            title="Open in browser tab"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              onExpand()
            }}
          >
            <Maximize size={16} aria-hidden />
          </button>
          <button
            type="button"
            className="br-ui-pip-btn"
            aria-label="Hide preview"
            title="Hide preview"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              onHide()
            }}
          >
            <X size={16} aria-hidden />
          </button>
        </div>
      ) : null}
    </div>
  )
}
