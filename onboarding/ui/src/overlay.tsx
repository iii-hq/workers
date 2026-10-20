import { Button, type Host, type OverlayRenderProps, Skeleton, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { Check, ChevronLeft, ChevronRight, Compass, Minus, X } from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'
import { RemoveOnboarding } from './page'
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
export function TourOverlay({ host, conversationId }: { host: Host } & OverlayRenderProps) {
  const view = useTourView()
  // The tour itself — progress, and the triggers its steps wait on — runs for
  // as long as the worker's script is loaded, whatever the card is doing: a
  // step the agent finishes while the card is closed still closes. Closed
  // only means nothing is drawn.
  const tour = useTour(host, conversationId ?? null)
  // Coming up, the card reads progress again: a step can close where this
  // tab was not listening (another tab, the console reporting an example's
  // answer before opening the tour).
  const { refresh } = tour
  const shown = view !== 'closed'
  useEffect(() => {
    if (shown) refresh()
  }, [shown, refresh])
  if (!shown) return null
  return <TourCard host={host} tour={tour} folded={view === 'minimized'} />
}

/** The card keeps 16px from the viewport's edges, and clears the masthead at the top. */
const EDGE = 16
const TOP_EDGE = 72

type Corner = 'bottom-right' | 'bottom-left' | 'top-right' | 'top-left'
const CORNERS: readonly Corner[] = ['bottom-right', 'bottom-left', 'top-right', 'top-left']

function TourCard({ host, tour, folded }: { host: Host; tour: TourController; folded: boolean }) {
  const { tour: loaded, openStep, framed } = tour
  const card = useRef<HTMLElement>(null)
  const corner = useCorner(card, folded)
  const titleId = useId()

  const index = loaded && openStep ? loaded.steps.indexOf(openStep) : -1
  const caption = openStep ? `${index + 1} · ${openStep.title}` : undefined
  useEffect(() => {
    // Folded is out of the way: the box goes with the card.
    if (folded) return
    showSpotlight(framed, caption)
    return hideSpotlight
  }, [framed, caption, folded])
  useEffect(() => disposeSpotlight, [])

  if (folded) {
    return <Pill tour={tour} corner={corner} cardRef={card} />
  }

  return (
    <section
      ref={card}
      className="ob-overlay"
      data-corner={corner}
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
      <header className="ob-overlay-head">
        <Compass className={`ob-overlay-mark ${uiClasses.icon}`} aria-hidden="true" />
        <span className="ob-overlay-name">{loaded?.title ?? 'Tour'}</span>
        {loaded && openStep ? (
          <span className="ob-overlay-count ob-num">
            {index + 1} / {loaded.steps.length}
          </span>
        ) : null}
        <Button
          variant="icon"
          size="icon"
          className="ob-link"
          aria-label="Fold the tour"
          title="Fold"
          onClick={minimizeTour}
        >
          <Minus className={uiClasses.icon} aria-hidden="true" />
        </Button>
        <Button
          variant="icon"
          size="icon"
          className="ob-link"
          aria-label="Close the tour"
          title="Close"
          onClick={closeTour}
        >
          <X className={uiClasses.icon} aria-hidden="true" />
        </Button>
      </header>
      {loaded ? <Progress tour={tour} /> : null}

      <div className="ob-overlay-scroll">
        {tour.error ? (
          <StatusPanel variant="alert" headline={tour.error} role="alert" />
        ) : !loaded ? (
          <div className="ob-loading" role="status" aria-busy="true" aria-label="Loading the tour">
            <Skeleton className="ob-skeleton ob-skeleton--bar" />
            <Skeleton className="ob-skeleton" />
          </div>
        ) : openStep ? (
          // Keyed on the step, so each one arrives instead of the text
          // swapping under the operator's eyes.
          <div key={openStep.id} className="ob-overlay-step">
            <h2 id={titleId} className="ob-overlay-title">
              <span className="ob-overlay-title-text">{openStep.title}</span>
              <StateChip state={tour.stateOf(openStep)} step={openStep} record={tour.records[openStep.id]} />
            </h2>
            <StepContent host={host} tour={tour} step={openStep} />
          </div>
        ) : (
          <Finished tour={tour} titleId={titleId} />
        )}
      </div>

      <footer className="ob-overlay-foot">
        <RemoveOnboarding host={host} onClose={closeTour} label="Remove tour" />
        {loaded && openStep ? <StepNav tour={tour} index={index} /> : null}
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
 * should move (`--ob-seg-order`): forward from the old position, or back from
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
    <ol className="ob-progress">
      {steps.map((step, index) => {
        const state = tour.stateOf(step)
        const here = index === reading && tour.open !== null
        return (
          <li key={step.id} className="ob-progress-item">
            <button
              type="button"
              className="ob-progress-seg"
              data-filled={index <= reading || undefined}
              data-done={state === 'complete' || undefined}
              style={{ '--ob-seg-order': order(index) } as React.CSSProperties}
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
    <span className="ob-overlay-nav">
      <Button
        variant="ghost"
        size="sm"
        className="ob-ghost"
        disabled={!prev}
        onClick={() => prev && tour.setOpen(prev.id)}
      >
        <ChevronLeft className={uiClasses.icon} aria-hidden="true" />
        Back
      </Button>
      <Button
        variant="ghost"
        size="sm"
        className="ob-ghost"
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
    <div className="ob-overlay-step">
      <h2 id={titleId} className="ob-overlay-title">
        <span className="ob-overlay-title-text">You have seen the whole tour</span>
        <span className="ob-chip" data-tone="ok">
          <Check className="ob-chip-mark" strokeWidth={2.5} aria-hidden="true" />
          All done
        </span>
      </h2>
      <p className="ob-body">Every step is done. Pick one in the bar above to read it again, or start over.</p>
      <div className="ob-actions">
        <Button className="ob-action" onClick={closeTour}>
          Close
        </Button>
        <Button variant="ghost" className="ob-ghost" onClick={tour.reset}>
          Restart
        </Button>
      </div>
    </div>
  )
}

/** The folded card: where the tour is, and one click back to it. */
function Pill({
  tour,
  corner,
  cardRef,
}: {
  tour: TourController
  corner: Corner
  cardRef: React.RefObject<HTMLElement | null>
}) {
  const steps = tour.tour?.steps ?? []
  const done = steps.filter((step) => tour.records[step.id]?.status === 'complete').length
  const waiting = Boolean(tour.active?.condition)
  return (
    <button
      ref={cardRef as React.RefObject<HTMLButtonElement>}
      type="button"
      className="ob-pill"
      data-corner={corner}
      onClick={showTour}
      aria-label={`Open the tour, ${done} of ${steps.length} steps done`}
    >
      <Compass className={uiClasses.icon} aria-hidden="true" />
      Tour
      {steps.length > 0 ? (
        <span className="ob-pill-count ob-num">
          {done}/{steps.length}
        </span>
      ) : null}
      {waiting ? <span className={`ob-chip-dot ${uiClasses.pulse}`} aria-hidden="true" /> : null}
    </button>
  )
}

/**
 * The corner the card sits in. It starts bottom right and stays put — a card
 * that chases every step is one more thing moving — until the spotlight
 * would land under it; then it takes the first corner that keeps the lit
 * element in view.
 */
function useCorner(card: React.RefObject<HTMLElement | null>, folded: boolean): Corner {
  const [corner, setCorner] = useState<Corner>('bottom-right')
  const cornerRef = useRef(corner)
  cornerRef.current = corner
  const lit = useRef<SpotlightRect | null>(null)

  useLayoutEffect(() => {
    const place = () => {
      const target = lit.current
      const element = card.current
      if (!target || !element || folded) return
      const size = { width: element.offsetWidth, height: element.offsetHeight }
      if (!overlaps(rectAt(cornerRef.current, size), target)) return
      const free = CORNERS.find((option) => !overlaps(rectAt(option, size), target))
      if (free && free !== cornerRef.current) setCorner(free)
    }
    const off = onSpotlightMove((rect) => {
      lit.current = rect
      place()
    })
    window.addEventListener('resize', place)
    return () => {
      off()
      window.removeEventListener('resize', place)
    }
  }, [card, folded])

  return corner
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
