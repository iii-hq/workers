import {
  type RefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { subscribeChatTurnStep } from '@/lib/chat-turn-nav'
import {
  currentTickIndex,
  nearestTickIndex,
  steppedTickIndex,
  TURN_RAIL_MIN_TURNS,
  TURN_RAIL_MIN_WIDTH_PX,
  type TurnSummary,
  type TurnTone,
  turnsFromMessages,
  type ViewportSegment,
  viewportSegment,
} from '@/lib/turn-rail'
import { cn } from '@/lib/utils'
import type { Message } from '@/types/chat'

interface TurnRailProps {
  messages: readonly Message[]
  container: RefObject<HTMLElement | null>
  content: RefObject<HTMLElement | null>
}

interface Tick extends TurnSummary {
  /** Scroll offset of the user message, in container scroll space. */
  offset: number
  /** The same as a fraction of the scroll height. */
  fraction: number
}

const TICK_TONE: Record<TurnTone, string> = {
  ink: 'bg-ink-ghost group-hover/tick:bg-ink',
  accent: 'bg-accent',
  alert: 'bg-alert',
}

const JUMP_MARGIN_PX = 16

function prefersReducedMotion(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
}

/**
 * A minimap of the conversation along the left edge: one tick per user turn
 * placed where that turn sits in the scroll range, a bar for the visible
 * window, a hover preview of the exchange, click to jump, drag to scrub.
 */
export function TurnRail({ messages, container, content }: TurnRailProps) {
  const turns = useMemo(() => turnsFromMessages(messages), [messages])
  const [ticks, setTicks] = useState<Tick[]>([])
  const [viewport, setViewport] = useState<ViewportSegment>({
    top: 0,
    height: 1,
  })
  const [wide, setWide] = useState(true)
  const [hover, setHover] = useState<number | null>(null)
  const [scrubbing, setScrubbing] = useState(false)
  const railRef = useRef<HTMLDivElement>(null)
  const ticksRef = useRef<Tick[]>([])
  ticksRef.current = ticks
  const frameRef = useRef<number | null>(null)

  const measure = useCallback(() => {
    const scroller = container.current
    const column = content.current
    if (!scroller || !column) return
    const scrollHeight = scroller.scrollHeight
    const base = scroller.getBoundingClientRect().top - scroller.scrollTop
    const next: Tick[] = []
    for (const turn of turns) {
      const element = column.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(turn.id)}"]`,
      )
      if (!element) continue
      const offset = Math.max(0, element.getBoundingClientRect().top - base)
      next.push({
        ...turn,
        offset,
        fraction: scrollHeight > 0 ? offset / scrollHeight : 0,
      })
    }
    setTicks(next)
    setViewport(
      viewportSegment(scroller.scrollTop, scroller.clientHeight, scrollHeight),
    )
    setWide(scroller.clientWidth >= TURN_RAIL_MIN_WIDTH_PX)
  }, [container, content, turns])

  useEffect(() => {
    measure()
    const column = content.current
    const scroller = container.current
    if (!column || !scroller) return
    const observer = new ResizeObserver(() => measure())
    observer.observe(column)
    observer.observe(scroller)
    return () => observer.disconnect()
  }, [measure, container, content])

  useEffect(() => {
    const scroller = container.current
    if (!scroller) return
    const onScroll = () => {
      if (frameRef.current !== null) return
      frameRef.current = requestAnimationFrame(() => {
        frameRef.current = null
        setViewport(
          viewportSegment(
            scroller.scrollTop,
            scroller.clientHeight,
            scroller.scrollHeight,
          ),
        )
      })
    }
    scroller.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      scroller.removeEventListener('scroll', onScroll)
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current)
      frameRef.current = null
    }
  }, [container])

  const jumpTo = useCallback(
    (index: number) => {
      const scroller = container.current
      const tick = ticksRef.current[index]
      if (!scroller || !tick) return
      scroller.scrollTo({
        top: Math.max(0, tick.offset - JUMP_MARGIN_PX),
        behavior: prefersReducedMotion() ? 'auto' : 'smooth',
      })
    },
    [container],
  )

  useEffect(
    () =>
      subscribeChatTurnStep((delta) => {
        const scroller = container.current
        if (!scroller) return
        const offsets = ticksRef.current.map(
          (tick) => tick.offset - JUMP_MARGIN_PX,
        )
        const current = currentTickIndex(scroller.scrollTop, offsets)
        const next = steppedTickIndex(current, delta, offsets.length)
        if (next >= 0) jumpTo(next)
      }),
    [container, jumpTo],
  )

  const fractionAt = useCallback((clientY: number): number => {
    const rail = railRef.current
    if (!rail) return 0
    const rect = rail.getBoundingClientRect()
    if (rect.height <= 0) return 0
    return Math.min(1, Math.max(0, (clientY - rect.top) / rect.height))
  }, [])

  const scrubTo = useCallback(
    (clientY: number) => {
      const scroller = container.current
      if (!scroller) return
      const fraction = fractionAt(clientY)
      scroller.scrollTop =
        fraction * (scroller.scrollHeight - scroller.clientHeight)
      const near = nearestTickIndex(
        fraction,
        ticksRef.current.map((tick) => tick.fraction),
      )
      setHover(near >= 0 ? near : null)
    },
    [container, fractionAt],
  )

  if (turns.length < TURN_RAIL_MIN_TURNS || !wide) return null

  const active = hover
  const preview = active !== null ? ticks[active] : undefined

  return (
    <div
      className="group/rail pointer-events-none absolute inset-y-3 left-0 z-10 w-9 pointer-fine:block hidden"
      data-turn-rail
    >
      <div
        ref={railRef}
        className={cn(
          'pointer-events-auto absolute inset-y-0 left-2 w-5 cursor-pointer touch-none select-none',
          'iii-ui-motion-control',
        )}
        onPointerDown={(event) => {
          if (event.button !== 0) return
          event.currentTarget.setPointerCapture(event.pointerId)
          setScrubbing(true)
          scrubTo(event.clientY)
        }}
        onPointerMove={(event) => {
          if (scrubbing) {
            scrubTo(event.clientY)
            return
          }
          const near = nearestTickIndex(
            fractionAt(event.clientY),
            ticks.map((tick) => tick.fraction),
          )
          setHover(near >= 0 ? near : null)
        }}
        onPointerUp={(event) => {
          if (!scrubbing) return
          event.currentTarget.releasePointerCapture(event.pointerId)
          setScrubbing(false)
        }}
        onPointerLeave={() => {
          if (!scrubbing) setHover(null)
        }}
      >
        <span
          aria-hidden="true"
          className="absolute inset-y-0 left-[9px] w-px bg-edge opacity-0 transition-opacity duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)] group-hover/rail:opacity-100"
        />
        <span
          aria-hidden="true"
          className="absolute left-[7px] w-[5px] rounded-full bg-ink/15 transition-[top,height] duration-[var(--motion-duration-fast)] ease-[var(--motion-ease-standard)] group-hover/rail:bg-ink/25"
          style={{
            top: `${viewport.top * 100}%`,
            height: `max(12px, ${viewport.height * 100}%)`,
          }}
        />
        {ticks.map((tick, index) => (
          <button
            key={tick.id}
            type="button"
            tabIndex={-1}
            aria-label={`turn ${index + 1}: ${tick.prompt}`}
            className="group/tick absolute left-0 flex h-3 w-5 -translate-y-1/2 items-center"
            style={{ top: `${tick.fraction * 100}%` }}
            onPointerEnter={() => setHover(index)}
            onClick={() => jumpTo(index)}
          >
            <span
              aria-hidden="true"
              className={cn(
                'block h-px rounded-full transition-[width,background-color] duration-[var(--motion-duration-control)] ease-[var(--motion-ease-standard)]',
                TICK_TONE[tick.tone],
                active === index ? 'w-5' : 'w-3',
                tick.tone === 'accent' && 'animate-pulse',
              )}
            />
            {tick.calls > 0 ? (
              <span
                aria-hidden="true"
                className="ml-0.5 size-1 rounded-full bg-ink-ghost"
              />
            ) : null}
          </button>
        ))}
      </div>
      {preview ? (
        <div
          role="tooltip"
          className="pointer-events-none absolute left-10 w-[min(360px,60vw)] -translate-y-1/2 rounded-md border border-edge bg-panel-raised px-3 py-2 font-sans text-[13px] shadow-lg"
          style={{
            top: `min(calc(100% - 48px), max(24px, ${preview.fraction * 100}%))`,
          }}
        >
          <p className="line-clamp-1 font-medium text-ink">{preview.prompt}</p>
          {preview.reply ? (
            <p className="mt-1 line-clamp-3 text-ink-faint">{preview.reply}</p>
          ) : null}
          {preview.calls > 0 ? (
            <p className="mt-1 text-ink-ghost">
              {preview.calls}{' '}
              {preview.calls === 1 ? 'function call' : 'function calls'}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
