import {
  Button,
  type Host,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  Skeleton,
  StatusPanel,
  uiClasses,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { Check, ChevronDown } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { disposeSpotlight, hideSpotlight, showSpotlight } from './spotlight'
import { StateChip, StepContent } from './step-content'
import { useTour } from './use-tour'

/**
 * The tour as a page, for a console without the overlay slot: one vertical
 * stepper, one open step.
 *
 * The list reads top to bottom as a path: a numbered dot per step on a
 * hairline that fills as steps complete, the title beside it, and — for the
 * step that is open — what to read and the one thing to do, in a panel that
 * grows under the title. Reached steps open and close in place; steps ahead
 * of the front are plain text until they are reached, so nothing is given
 * away early. A console with the slot shows the card instead (overlay.tsx);
 * both run on `useTour`.
 */
export function OnboardingPage({ host, onRequestClose, conversationId }: { host: Host } & PageRenderProps) {
  const tour = useTour(host, conversationId)
  const { tour: loaded, records, error, open, setOpen, active, framed } = tour
  const removeButton = <RemoveOnboarding host={host} onClose={onRequestClose} />

  const caption = tour.openStep && loaded ? `${loaded.steps.indexOf(tour.openStep) + 1} · ${tour.openStep.title}` : undefined
  useEffect(() => {
    showSpotlight(framed, caption)
    return hideSpotlight
  }, [framed, caption])
  useEffect(() => disposeSpotlight, [])

  if (error) {
    return (
      <Frame onClose={onRequestClose} footer={removeButton}>
        <StatusPanel variant="alert" headline={error} role="alert" />
      </Frame>
    )
  }
  if (!loaded) {
    return (
      <Frame onClose={onRequestClose} footer={removeButton}>
        <div className="ob-loading" role="status" aria-busy="true" aria-label="Loading the tour">
          <Skeleton className="ob-skeleton ob-skeleton--bar" />
          <Skeleton className="ob-skeleton" />
          <Skeleton className="ob-skeleton" />
          <Skeleton className="ob-skeleton" />
        </div>
      </Frame>
    )
  }

  const total = loaded.steps.length
  const done = loaded.steps.filter((step) => records[step.id]?.status === 'complete').length
  const activeIndex = active ? loaded.steps.indexOf(active) : -1

  return (
    <Frame title={loaded.title} description={loaded.description} onClose={onRequestClose} footer={removeButton}>
      <header className="ob-head">
        <p className="ob-head-count" role="status">
          {done === total ? (
            'All steps done'
          ) : (
            <>
              Step <span className="ob-num">{activeIndex + 1}</span> of <span className="ob-num">{total}</span>
            </>
          )}
          <span className="ob-head-done">
            {' · '}
            <span className="ob-num">{done}</span> done
          </span>
        </p>
        <Button variant="ghost" size="sm" className="ob-ghost" onClick={tour.reset} disabled={done === 0}>
          Restart
        </Button>
      </header>

      <ol className="ob-steps">
        {loaded.steps.map((step, index) => {
          const record = records[step.id]
          const state = tour.stateOf(step)
          // A step opens once it is reached. Reading ahead would give away a
          // box the operator has not been shown yet.
          const reached = state !== 'pending'
          const isOpen = reached && open === step.id
          const last = index === total - 1
          const indicator = (
            <span className="ob-indicator" data-state={state} aria-hidden="true">
              <span className="ob-indicator-num">{index + 1}</span>
              <Check className="ob-indicator-check" strokeWidth={2.5} />
              {state === 'active' ? <span className={`ob-indicator-pulse ${uiClasses.pulse}`} /> : null}
            </span>
          )
          return (
            <li
              key={step.id}
              className="ob-step"
              data-state={state}
              data-open={isOpen || undefined}
              aria-current={state === 'active' ? 'step' : undefined}
            >
              {last ? null : (
                <span className="ob-line" data-done={state === 'complete' || undefined} aria-hidden="true" />
              )}
              {reached ? (
                <button
                  type="button"
                  className="ob-row"
                  aria-expanded={isOpen}
                  onClick={() => setOpen(isOpen ? null : step.id)}
                >
                  {indicator}
                  <span className="ob-title">{step.title}</span>
                  <StateChip state={state} step={step} record={record} />
                  <ChevronDown className={`ob-chevron ${uiClasses.icon}`} aria-hidden="true" />
                </button>
              ) : (
                <div className="ob-row">
                  {indicator}
                  <span className="ob-title">{step.title}</span>
                  <span className="ob-sr">not reached yet</span>
                </div>
              )}
              {reached ? (
                // Mounted while the step is reached and folded with a grid
                // track, so opening is a transition that can reverse midway
                // rather than an entrance that restarts from nothing.
                <div className="ob-panel" data-open={isOpen || undefined} inert={!isOpen}>
                  <div className="ob-panel-clip">
                    <div className="ob-panel-body ob-card">
                      <StepContent host={host} tour={tour} step={step} />
                    </div>
                  </div>
                </div>
              ) : null}
            </li>
          )
        })}
      </ol>
    </Frame>
  )
}

/**
 * The way out, on every step: take the tour's worker out of the project and
 * close the card (or the pane). The compose file is named from what
 * the daemon loaded, so a daemon started elsewhere still finds its project.
 */
export function RemoveOnboarding({
  host,
  onClose,
  label = 'Close and remove the tour',
}: {
  host: Host
  onClose?: () => void
  /** The card is narrow, so it says less. */
  label?: string
}) {
  const [removing, setRemoving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const remove = useCallback(async () => {
    setRemoving(true)
    setError(null)
    try {
      const list = await host.iii
        .trigger<{ projects?: { file?: string; containers?: { container?: string }[] }[] }>('compose::list', {})
        .catch(() => null)
      // With several projects loaded, the one running this worker.
      const withFile = (list?.projects ?? []).filter((project) => project.file)
      const file = (
        withFile.find((project) => project.containers?.some((entry) => entry.container === 'onboarding')) ?? withFile[0]
      )?.file
      await host.iii.trigger('compose::remove', {
        ...(file ? { file } : {}),
        workers: ['onboarding'],
      })
    } catch (err) {
      setError(`Could not remove the onboarding worker: ${errorMessage(err)}`)
      setRemoving(false)
      return
    }
    if (onClose) onClose()
    else await host.iii.trigger('console::workspace::close', { screen: 'ext:onboarding' }).catch(() => undefined)
  }, [host, onClose])
  return (
    <>
      {error ? <StatusPanel variant="alert" headline={error} role="alert" /> : null}
      <Button variant="ghost" size="sm" className="ob-ghost" onClick={remove} disabled={removing}>
        {removing ? 'Removing…' : label}
      </Button>
    </>
  )
}

function Frame({
  title = 'onboarding',
  description,
  onClose,
  footer,
  children,
}: {
  title?: string
  description?: string
  /** The console's own pane close, from `PageRenderProps.onRequestClose`.
      Absent when the page is not rendered in a closable pane. */
  onClose?: () => void
  /** Pinned under the scroller, right-aligned: the way out. */
  footer?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <PageShell>
      <PageHeader title={title} description={description} onClose={onClose} />
      <PageMain>
        {/* `PageMain` is `overflow-hidden`, and so is every other piece of the
            page chrome — a page that wants to scroll has to say so. Without
            this the tour is simply cut off at the bottom of the pane, with no
            way to reach the last steps. The scroller sits OUTSIDE the centred
            column so the scrollbar rides the pane's edge rather than the
            text's. */}
        <div className="ob-scroll">
          {/* Centred column: the pane is often narrow beside a chat, but a
              page wide enough to be a whole tab should not leave the list
              stranded on one edge. */}
          <div className="ob-column">{children}</div>
        </div>
        {footer ? <div className="ob-footer">{footer}</div> : null}
      </PageMain>
    </PageShell>
  )
}

