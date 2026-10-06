import { Card, type Host, IconButton } from '@iii-dev/console-ui'
import { usePaneState } from '@iii-dev/console-ui/hooks'
import { Maximize, X } from 'lucide-react'
import {
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { ACTIVITY, DEFAULT_TENANT, DEVICE_CHANGED, OPEN, SimApi } from '../lib/api'
import { useLiveScreen } from '../page/live'
import {
  type Bounds,
  boundsFor,
  clampPoint,
  DRAG_THRESHOLD,
  decayVelocity,
  isStill,
  overshoot,
  type Point,
  resizedFromBottomRight,
  rubberBandPoint,
  type Sample,
  velocityFromSamples,
} from './motion'
import { type Limits, overlayLimits, pinchDistance, pinchWidth, settleWidth, toggledWidth } from './size'
import { bringToFront, dismissCard, hideCard, openSimulatorPane, overlayCards, showCard, subscribeOverlay } from './store'

/**
 * The live preview, as the browser worker's: small picture-in-picture phones
 * of the simulators an agent (or Xcode, or a terminal) boots, fed by the same
 * live view the Simulators page uses. The group sits in a corner (top-right
 * on touch layouts, bottom-right with a pointer) until dragged elsewhere —
 * one finger or the mouse moves it, a released swipe coasts and settles
 * inside the viewport, a pinch resizes it. Width and spot stick across
 * reloads.
 *
 * Several simulators stack like a deck: the newest in front, the others
 * tilted behind it. Tapping the front fans the deck out sideways (and shows
 * its controls: open in the Simulators page, hide); tapping any card in the
 * fan brings it to the front and folds the deck again. A card goes away when
 * its simulator shuts down or is deleted.
 *
 * Only the `default` tenant previews here: that is where the console's
 * agents work, and on a Mac shared as an API other tenants' boots are not
 * the operator's to watch.
 */

const WIDTH_KEY = 'ios-simulator-ui:overlay:width'
const POSITION_KEY = 'ios-simulator-ui:overlay:position'
const POINTER_LAYOUT = '(hover: hover) and (pointer: fine)'
const TOUCH_WIDTH = 120
const POINTER_WIDTH = 180
const defaultWidth = () => (window.matchMedia(POINTER_LAYOUT).matches ? POINTER_WIDTH : TOUCH_WIDTH)
/** Matches `--motion-duration-panel`: the element unmounts after its exit ran. */
const CLOSE_MS = 220
/** Longest frame a fling integrates over (a background tab's rAF pause). */
const MAX_FRAME_MS = 48
/** Past this far outside the viewport a fling stops and snaps back. */
const MAX_OVERSHOOT = 80
const VELOCITY_SAMPLES = 6
/** How much of a card's width the next one in the fan is offset by. */
const FAN_OVERLAP = 0.6
/** Until the first frame says otherwise: an iPhone 17 Pro. */
const DEFAULT_ASPECT = '1206 / 2622'

/** Folded tilt of the card `depth` places behind the front. */
function foldedTilt(depth: number): number {
  if (depth === 0) return 0
  return (depth % 2 === 1 ? -1 : 1) * (2 + 3 * depth)
}

const isPoint = (v: unknown): v is Point =>
  typeof v === 'object' && v !== null && typeof (v as Point).x === 'number' && typeof (v as Point).y === 'number'

const limitsNow = () => overlayLimits(window.innerWidth, window.innerHeight)

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

interface DeviceEvent {
  tenant?: unknown
  udid?: unknown
  state?: unknown
  previous_state?: unknown
  preview?: unknown
  function?: unknown
}

/**
 * Show a card when a simulator boots or a caller (an agent) drives it, drop
 * it when it stops. Only the `default` tenant: the console's own simulators.
 */
function useDeviceEvents(host: Host) {
  const instance = useId().replace(/[^a-zA-Z0-9]/g, '')
  useEffect(() => {
    const bind = (type: string, name: string, onEvent: (key: string, e: DeviceEvent) => void) => {
      const handler = `iii::ios-simulator-ui::overlay-${name}::${instance}`
      const off = host.iii.on(handler, (payload: unknown) => {
        const e = (payload ?? {}) as DeviceEvent
        if (typeof e.tenant === 'string' && typeof e.udid === 'string') onEvent(`${e.tenant}/${e.udid}`, e)
      })
      const unbind = host.iii.registerTrigger({
        type,
        function_id: `${handler}::${host.iii.browserId}`,
        config: { tenant: DEFAULT_TENANT },
      })
      return () => {
        unbind()
        off()
      }
    }
    const offs: Array<() => void> = []
    try {
      offs.push(
        bind(DEVICE_CHANGED, 'device', (key, e) => {
          if (e.state !== 'Booted') hideCard(key)
          else if (e.previous_state !== 'Booted' && e.preview !== false) showCard(key, true)
        }),
      )
      offs.push(bind(ACTIVITY, 'activity', (key, e) => showCard(key, e.function === OPEN)))
    } catch (err) {
      console.warn('[ios-simulator-ui] preview binding failed', err)
    }
    return () => {
      for (const off of offs) off()
    }
  }, [host, instance])
}

export function SimulatorOverlay({ host }: { host: Host }) {
  const cards = useSyncExternalStore(subscribeOverlay, overlayCards)
  useDeviceEvents(host)

  // The cards outlive an emptied deck by one exit transition.
  const [shown, setShown] = useState<readonly string[]>([])
  useEffect(() => {
    if (cards.length > 0) {
      setShown(cards)
      return
    }
    const timer = window.setTimeout(() => setShown([]), CLOSE_MS)
    return () => window.clearTimeout(timer)
  }, [cards])
  const front = cards[cards.length - 1] ?? null

  // Cards with a picture; the deck reveals once the front one has its own.
  const [ready, setReady] = useState<ReadonlySet<string>>(() => new Set())
  const onReady = useCallback((key: string, hasFrame: boolean) => {
    setReady((current) => {
      if (current.has(key) === hasFrame) return current
      const next = new Set(current)
      if (hasFrame) next.add(key)
      else next.delete(key)
      return next
    })
  }, [])
  const open = front !== null && ready.has(front)

  const [expanded, setExpanded] = useState(false)
  useEffect(() => {
    if (cards.length === 0) setExpanded(false)
  }, [cards.length])
  const [fan, setFan] = useState({ dir: -1, step: 0 })

  const rootRef = useRef<HTMLElement>(null)
  const [storedWidth, setStoredWidth] = usePaneState<unknown>(WIDTH_KEY, null)
  const [width, setWidth] = useState(() =>
    typeof storedWidth === 'number' && Number.isFinite(storedWidth) && storedWidth > 0
      ? storedWidth
      : defaultWidth(),
  )
  const widthRef = useRef(width)
  widthRef.current = width
  const [storedPosition, setStoredPosition] = usePaneState<unknown>(POSITION_KEY, null)
  const [position, setPosition] = useState<Point>(() => (isPoint(storedPosition) ? storedPosition : { x: 0, y: 0 }))
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

  const settle = useCallback(
    (next: Point) => {
      setMoving(false)
      setPosition(next)
      setStoredPosition(next)
    },
    [setStoredPosition],
  )

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
        if (drag.current?.moved) settle(clampPoint(drag.current.raw, drag.current.bounds))
        drag.current = null
        const [a, b] = [...map.values()]
        pinch.current = {
          startDistance: pinchDistance(a, b) || 1,
          startWidth: widthRef.current,
          limits: limitsNow(),
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
          setWidth(pinchWidth(gesture.startWidth, pinchDistance(p, q) / gesture.startDistance, gesture.limits))
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
          const settled = settleWidth(widthRef.current, gesture.limits)
          setWidth(settled)
          setStoredWidth(settled)
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
    [currentBounds, settle, setStoredWidth, startFling, stopFling],
  )

  // A resized viewport (rotation, a window drag) keeps the box in view.
  useEffect(() => {
    const onResize = () => {
      if (drag.current || fling.current !== null) return
      const bounds = currentBounds()
      if (!bounds) return
      const clamped = clampPoint(positionRef.current, bounds)
      if (clamped.x !== positionRef.current.x || clamped.y !== positionRef.current.y) settle(clamped)
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
    const step = cardCount > 1 ? Math.min(width * FAN_OVERLAP, room / (cardCount - 1)) : 0
    setFan({ dir, step })
  }, [expanded, width, cardCount])

  // Folded: the front toggles the fan, a peeking card comes forward. Fanned
  // out: any card comes forward and the deck folds behind it.
  const onCardClick = useCallback((key: string, isFront: boolean) => {
    if (gesturedRef.current) {
      gesturedRef.current = false
      return
    }
    if (!isFront) bringToFront(key)
    setExpanded((value) => (isFront ? !value : false))
  }, [])

  // With a mouse there is no pinch: a double-click alternates the default
  // width and twice it (the width transition animates it).
  const onDoubleClick = useCallback(
    (event: ReactMouseEvent<HTMLElement>) => {
      if (!window.matchMedia(POINTER_LAYOUT).matches) return
      if (!(event.target as Element).closest('.ios-ui-pip-surface')) return
      const next = toggledWidth(widthRef.current, POINTER_WIDTH, limitsNow())
      // Growing from the anchored corner can push the far edges out of view:
      // the box moves as it grows so all of it stays visible.
      const node = rootRef.current
      if (node) {
        const bounds = boundsFor(resizedFromBottomRight(node.getBoundingClientRect(), next), positionRef.current, {
          width: window.innerWidth,
          height: window.innerHeight,
        })
        const clamped = clampPoint(positionRef.current, bounds)
        if (clamped.x !== positionRef.current.x || clamped.y !== positionRef.current.y) settle(clamped)
      }
      setWidth(next)
      setStoredWidth(next)
    },
    [setStoredWidth, settle],
  )

  if (shown.length === 0) return null

  return (
    <section
      ref={rootRef}
      className={['ios-ui-pip', pinching && 'is-pinching', moving && 'is-moving'].filter(Boolean).join(' ')}
      data-expanded={expanded ? 'true' : 'false'}
      aria-label="Live simulator preview"
      style={{
        width,
        transform: `translate(${position.x}px, ${position.y}px)`,
        ['--fan-dir' as string]: fan.dir,
        ['--fan-step' as string]: `${fan.step}px`,
      }}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
    >
      <div className="ios-ui-pip-deck ios-ui-pip-reveal" data-open={open ? 'true' : 'false'}>
        {shown.map((key, index) => {
          const depth = shown.length - 1 - index
          return (
            <PreviewCard
              key={key}
              host={host}
              cardKey={key}
              depth={depth}
              live={depth === 0 || expanded}
              expanded={expanded}
              width={width}
              onReady={onReady}
              onClick={onCardClick}
              onExpand={() => {
                setExpanded(false)
                openSimulatorPane(host, key)
              }}
              onHide={() => {
                setExpanded(false)
                dismissCard(key)
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
  /** `<tenant>/<udid>`. */
  cardKey: string
  /** 0 is the front card; higher sits further back in the deck. */
  depth: number
  /** Stream frames; a folded-away card keeps its last picture instead. */
  live: boolean
  expanded: boolean
  width: number
  onReady: (key: string, hasFrame: boolean) => void
  onClick: (key: string, isFront: boolean) => void
  onExpand: () => void
  onHide: () => void
}

function PreviewCard({ host, cardKey, depth, live, expanded, width, onReady, onClick, onExpand, onHide }: CardProps) {
  const [tenant, udid] = cardKey.split('/')
  const api = useMemo(() => new SimApi(host.iii, tenant), [host, tenant])
  const { screen } = useLiveScreen(host, api, udid, live)
  const hasFrame = screen !== null
  useEffect(() => {
    onReady(cardKey, hasFrame)
    return () => onReady(cardKey, false)
  }, [onReady, cardKey, hasFrame])

  const isFront = depth === 0
  return (
    <Card
      className="ios-ui-pip-card"
      data-front={isFront ? 'true' : 'false'}
      data-ready={hasFrame ? 'true' : 'false'}
      style={{
        ['--depth' as string]: depth,
        ['--tilt' as string]: `${foldedTilt(depth)}deg`,
        width,
        aspectRatio: screen ? `${screen.deviceWidth} / ${screen.deviceHeight}` : DEFAULT_ASPECT,
        zIndex: 100 - depth,
      }}
    >
      <button
        type="button"
        className="ios-ui-pip-surface"
        aria-label={isFront ? (expanded ? 'Fold the preview' : 'Fan the preview out') : 'Bring this simulator to the front'}
        aria-expanded={isFront ? expanded : undefined}
        onClick={() => onClick(cardKey, isFront)}
      >
        {screen ? <img src={screen.src} alt="" draggable={false} /> : null}
      </button>
      {isFront ? (
        <div className="ios-ui-pip-controls" data-open={expanded ? 'true' : 'false'}>
          <IconButton
            label="Open in Simulators"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              onExpand()
            }}
          >
            <Maximize size={16} aria-hidden />
          </IconButton>
          <IconButton
            label="Hide preview"
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation()
              onHide()
            }}
          >
            <X size={16} aria-hidden />
          </IconButton>
        </div>
      ) : null}
    </Card>
  )
}
