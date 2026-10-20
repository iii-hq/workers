import { Button, type Host, type OverlayRenderProps, Skeleton, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { Check, ChevronLeft, ChevronRight, Compass, Minus, X } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { RemoveLearnIii } from './page'
import { disposeSpotlight, hideSpotlight, onSpotlightMove, type SpotlightRect, showSpotlight } from './spotlight'
import { StateChip, StepContent } from './step-content'
import { type TourController, useTour } from './use-tour'
import { closeTour, minimizeTour, showTour, useTourView } from './visibility'

/**
 * The tour as a card floating over the console (the `overlays` slot), so
 * following it never rearranges the workspace being taught: no pane opens
 * for it, nothing is resized, and the operator's own layout stays theirs.
 *
 * It does not take focus. The console's overlay layer lets clicks through
 * and the card opts back in for itself only; it is a non-modal dialog that
 * nothing focuses on mount, so typing in the chat or clicking a tab works
 * the same with the card up. One step shows at a time, with Back and Next
 * across the steps already reached, and the spotlight names the element the
 * step is about. It folds to a pill, and closes until it is asked for again.
 */
export function TourOverlay({ host }: { host: Host } & OverlayRenderProps) {
  const view = useTourView()
  // The tour itself — progress, and the triggers its steps wait on — runs for
  // as long as the worker's script is loaded, whatever the card is doing: a
  // step the agent finishes while the card is closed still closes. Closed
  // only means nothing is drawn.
  const tour = useTour(host)
  // Coming up, the card reads progress again: a step can close where this
  // tab was not listening (another tab, the console reporting an example's
  // answer before opening the tour).
  const { refresh } = tour
  const shown = view !== 'closed'
  useEffect(() => {
    if (shown) refresh()
  }, [shown, refresh])
  // The pill's own close: the card goes away, and a note says how to get it back.
  const [hint, setHint] = useState(false)
  useEffect(() => {
    if (!hint) return
    const timer = window.setTimeout(() => setHint(false), HINT_MS)
    return () => window.clearTimeout(timer)
  }, [hint])
  const dismiss = () => {
    closeTour()
    setHint(true)
  }
  return (
    <>
      {shown ? <TourCard host={host} tour={tour} folded={view === 'minimized'} onDismiss={dismiss} /> : null}
      {hint ? (
        <div className="learn-hint" role="status">
          Press {paletteKey()} and type “learn iii” to start the tour.
        </div>
      ) : null}
    </>
  )
}

const HINT_MS = 8000

/** The console's palette shortcut, named for this platform. */
function paletteKey(): string {
  return /mac|iphone|ipad/i.test(navigator.platform) ? '⌘K' : 'Ctrl+K'
}


/** The card keeps 16px from the viewport's edges, and clears the masthead at the top. */
const EDGE = 16
const TOP_EDGE = 72

type Corner = 'bottom-right' | 'bottom-left' | 'top-right' | 'top-left'
const CORNERS: readonly Corner[] = ['bottom-right', 'bottom-left', 'top-right', 'top-left']

function TourCard({
  host,
  tour,
  folded,
  onDismiss,
}: {
  host: Host
  tour: TourController
  folded: boolean
  onDismiss: () => void
}) {
  const { tour: loaded, openStep, framed } = tour
  const card = useRef<HTMLElement>(null)
  const { corner, place, settled } = usePlacement(card, folded)
  const titleId = useId()
  // Step 0: nothing is lit and nothing is counted until the operator says yes.
  const intro = Boolean(loaded) && tour.records.intro?.status !== 'complete'

  const index = loaded && openStep ? loaded.steps.indexOf(openStep) : -1
  const caption = openStep ? `${index + 1} · ${openStep.title}` : undefined
  useEffect(() => {
    // Folded is out of the way: the box goes with the card.
    if (folded || intro) return
    showSpotlight(framed, caption)
    return hideSpotlight
  }, [framed, caption, folded, intro])
  useEffect(() => disposeSpotlight, [])

  if (folded) {
    return <Pill tour={tour} corner={corner} cardRef={card} onDismiss={onDismiss} />
  }

  return (
    <section
      ref={card}
      className="learn-overlay"
      data-corner={corner}
      data-settled={settled || undefined}
      style={place ? { translate: `${place.left}px ${place.top}px` } : undefined}
      role="dialog"
      aria-modal="false"
      aria-labelledby={titleId}
      onKeyDown={(event) => {
        // Escape folds the card when the operator is inside it; outside, the
        // key belongs to whatever has the focus.
        if (event.key === 'Escape') {
          event.stopPropagation()
          minimizeTour()
        }
      }}
    >
      <header className="learn-overlay-head">
        <Compass className={`learn-overlay-mark ${uiClasses.icon}`} aria-hidden="true" />
        <span className="learn-overlay-name">{loaded?.title ?? 'Tour'}</span>
        {loaded && openStep && !intro ? (
          <span className="learn-overlay-count learn-num">
            {index + 1} / {loaded.steps.length}
          </span>
        ) : null}
        <Button
          variant="icon"
          size="icon"
          className="learn-link"
          aria-label="Fold the tour"
          title="Fold"
          onClick={minimizeTour}
        >
          <Minus className={uiClasses.icon} aria-hidden="true" />
        </Button>
        <Button
          variant="icon"
          size="icon"
          className="learn-link"
          aria-label="Close the tour"
          title="Close"
          onClick={closeTour}
        >
          <X className={uiClasses.icon} aria-hidden="true" />
        </Button>
      </header>
      {loaded && !intro ? <Progress tour={tour} /> : null}

      <div className="learn-overlay-scroll">
        {tour.error ? (
          <StatusPanel variant="alert" headline={tour.error} role="alert" />
        ) : !loaded ? (
          <div className="learn-loading" role="status" aria-busy="true" aria-label="Loading the tour">
            <Skeleton className="learn-skeleton learn-skeleton--bar" />
            <Skeleton className="learn-skeleton" />
          </div>
        ) : intro && loaded ? (
          <div className="learn-overlay-step">
            <h2 id={titleId} className="learn-overlay-title">
              <span className="learn-overlay-title-text">{loaded.intro.title}</span>
            </h2>
            <p className="learn-body">{loaded.intro.body}</p>
            <div className="learn-actions">
              <Button className="learn-action" onClick={() => tour.complete('intro')}>
                Show me
              </Button>
              <Button variant="ghost" className="learn-ghost" onClick={minimizeTour}>
                Not now
              </Button>
            </div>
          </div>
        ) : openStep ? (
          // Keyed on the step, so each one arrives instead of the text
          // swapping under the operator's eyes.
          <div key={openStep.id} className="learn-overlay-step">
            <h2 id={titleId} className="learn-overlay-title">
              <span className="learn-overlay-title-text">{openStep.title}</span>
              <StateChip state={tour.stateOf(openStep)} step={openStep} record={tour.records[openStep.id]} />
            </h2>
            <StepContent host={host} tour={tour} step={openStep} />
          </div>
        ) : (
          <Finished tour={tour} titleId={titleId} />
        )}
      </div>

      <footer className="learn-overlay-foot">
        {/* Removing the worker is the last step's business, and the finished card's. */}
        {loaded && !intro && (!openStep || index === loaded.steps.length - 1) ? (
          <RemoveLearnIii host={host} onClose={closeTour} label="Remove this worker" />
        ) : null}
        {loaded && openStep && !intro ? <StepNav tour={tour} index={index} /> : null}
      </footer>
    </section>
  )
}

/**
 * One segment per step, filled up to the step being read — so Back drains the
 * bar and Next fills it — with steps done ahead of that point on a faint
 * track. A reached step's segment opens it; the ones ahead are not buttons
 * yet.
 *
 * Between two reads, the segments that change are numbered in the order they
 * should move (`--learn-seg-order`): forward from the old position, or back from
 * it, so the fill travels instead of every segment switching at once. A jump
 * across many steps shares the same short sweep rather than a longer one.
 */
function Progress({ tour }: { tour: TourController }) {
  const steps = tour.tour?.steps ?? []
  // No step open is the finished card: the whole bar is filled.
  const reading = tour.open ? steps.findIndex((step) => step.id === tour.open) : steps.length - 1
  const last = useRef(reading)
  const from = last.current
  useEffect(() => {
    last.current = reading
  }, [reading])
  const span = Math.abs(reading - from)
  // At most a six-segment sweep, however far the jump.
  const pace = span > 6 ? 6 / span : 1
  const order = (index: number): number => {
    if (reading > from && index > from && index <= reading) return (index - from - 1) * pace
    if (reading < from && index > reading && index <= from) return (from - index) * pace
    return 0
  }
  return (
    <ol className="learn-progress">
      {steps.map((step, index) => {
        const state = tour.stateOf(step)
        const here = index === reading && tour.open !== null
        return (
          <li key={step.id} className="learn-progress-item">
            <button
              type="button"
              className="learn-progress-seg"
              data-filled={index <= reading || undefined}
              data-done={state === 'complete' || undefined}
              style={{ '--learn-seg-order': order(index) } as React.CSSProperties}
              disabled={state === 'pending'}
              aria-current={here ? 'step' : undefined}
              aria-label={`Step ${index + 1}: ${step.title}${state === 'complete' ? ', done' : ''}`}
              title={state === 'pending' ? undefined : step.title}
              onClick={() => tour.setOpen(step.id)}
            />
          </li>
        )
      })}
    </ol>
  )
}

/**
 * Back and Next across the steps already reached. Next stops at the front:
 * a step ahead of it is shown when the one before it is done, the same rule
 * as the list, so the card never gives a step away early.
 */
function StepNav({ tour, index }: { tour: TourController; index: number }) {
  const steps = tour.tour?.steps ?? []
  const prev = index > 0 ? steps[index - 1] : undefined
  const next = steps[index + 1]
  const nextReached = next ? tour.stateOf(next) !== 'pending' : false
  return (
    <span className="learn-overlay-nav">
      <Button
        variant="ghost"
        size="sm"
        className="learn-ghost"
        disabled={!prev}
        onClick={() => prev && tour.setOpen(prev.id)}
      >
        <ChevronLeft className={uiClasses.icon} aria-hidden="true" />
        Back
      </Button>
      <Button
        variant="ghost"
        size="sm"
        className="learn-ghost"
        disabled={!nextReached}
        onClick={() => next && tour.setOpen(next.id)}
      >
        Next
        <ChevronRight className={uiClasses.icon} aria-hidden="true" />
      </Button>
    </span>
  )
}

function Finished({ tour, titleId }: { tour: TourController; titleId: string }) {
  return (
    <div className="learn-overlay-step">
      <h2 id={titleId} className="learn-overlay-title">
        <span className="learn-overlay-title-text">You have seen the whole tour</span>
        <span className="learn-chip" data-tone="ok">
          <Check className="learn-chip-mark" strokeWidth={2.5} aria-hidden="true" />
          All done
        </span>
      </h2>
      <p className="learn-body">Every step is done. Pick one in the bar above to read it again, or start over.</p>
      <div className="learn-actions">
        <Button className="learn-action" onClick={closeTour}>
          Close
        </Button>
        <Button variant="ghost" className="learn-ghost" onClick={tour.reset}>
          Restart
        </Button>
      </div>
    </div>
  )
}

/**
 * The folded card: where the tour is, and one click back to it. The count is
 * the open card's: the step up next (the last one once all are done), of all.
 * Its own close puts the card away and says how to bring it back.
 */
function Pill({
  tour,
  corner,
  cardRef,
  onDismiss,
}: {
  tour: TourController
  corner: Corner
  cardRef: React.RefObject<HTMLElement | null>
  onDismiss: () => void
}) {
  const steps = tour.tour?.steps ?? []
  const at = tour.active ? steps.indexOf(tour.active) + 1 : steps.length
  const waiting = Boolean(tour.active?.condition)
  return (
    <span ref={cardRef as React.RefObject<HTMLSpanElement>} className="learn-pill" data-corner={corner}>
      <button
        type="button"
        className="learn-pill-open"
        onClick={showTour}
        aria-label={`Open the tour, step ${at} of ${steps.length}`}
      >
        <Compass className={uiClasses.icon} aria-hidden="true" />
        Tour
        {steps.length > 0 ? (
          <span className="learn-pill-count learn-num">
            {at}/{steps.length}
          </span>
        ) : null}
        {waiting ? <span className={`learn-chip-dot ${uiClasses.pulse}`} aria-hidden="true" /> : null}
      </button>
      <button type="button" className="learn-pill-close" onClick={onDismiss} aria-label="Close the tour" title="Close">
        <X className={uiClasses.icon} aria-hidden="true" />
      </button>
    </span>
  )
}

type Place = { left: number; top: number }

/**
 * Where the card sits. It starts bottom right and stays put — a card that
 * chases every step is one more thing moving — until the spotlight would
 * land under it. Then it docks just left of the lit element, level with its
 * top, when there is room; otherwise it takes the first corner that keeps
 * the element in view. The card is placed in pixels (`translate`), so the
 * move is a slide the operator can follow; the pill keeps its corner.
 */
function usePlacement(
  card: React.RefObject<HTMLElement | null>,
  folded: boolean,
): { corner: Corner; place: Place | null; settled: boolean } {
  const [corner, setCorner] = useState<Corner>('bottom-right')
  const [place, setPlace] = useState<Place | null>(null)
  const [settled, setSettled] = useState(false)
  const cornerRef = useRef(corner)
  cornerRef.current = corner
  const lit = useRef<SpotlightRect | null>(null)

  useLayoutEffect(() => {
    if (folded) {
      setPlace(null)
      setSettled(false)
      return
    }
    const put = () => {
      const element = card.current
      if (!element) return
      const size = { width: element.offsetWidth, height: element.offsetHeight }
      const target = lit.current
      let at = rectAt(cornerRef.current, size)
      if (target && overlaps(at, target)) {
        const left = target.left - EDGE - size.width
        if (left >= EDGE) {
          at = { ...at, left, top: clamp(target.top, TOP_EDGE, window.innerHeight - EDGE - size.height) }
        } else {
          const free = CORNERS.find((option) => !overlaps(rectAt(option, size), target)) ?? cornerRef.current
          if (free !== cornerRef.current) setCorner(free)
          at = rectAt(free, size)
        }
      }
      setPlace((current) => (current && current.left === at.left && current.top === at.top ? current : at))
    }
    put()
    // The first place is where the card appears; the slide starts after that.
    const frame = window.requestAnimationFrame(() => setSettled(true))
    const off = onSpotlightMove((rect) => {
      lit.current = rect
      put()
    })
    const grew = new ResizeObserver(put)
    if (card.current) grew.observe(card.current)
    window.addEventListener('resize', put)
    return () => {
      window.cancelAnimationFrame(frame)
      off()
      grew.disconnect()
      window.removeEventListener('resize', put)
    }
  }, [card, folded])

  return { corner, place, settled }
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), Math.max(low, high))
}

function rectAt(corner: Corner, size: { width: number; height: number }): SpotlightRect {
  const right = corner.endsWith('right')
  const bottom = corner.startsWith('bottom')
  return {
    left: right ? window.innerWidth - EDGE - size.width : EDGE,
    top: bottom ? window.innerHeight - EDGE - size.height : TOP_EDGE,
    width: size.width,
    height: size.height,
  }
}

function overlaps(a: SpotlightRect, b: SpotlightRect): boolean {
  return a.left < b.left + b.width && b.left < a.left + a.width && a.top < b.top + b.height && b.top < a.top + a.height
}
