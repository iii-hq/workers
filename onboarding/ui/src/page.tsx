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
export function LearnIiiPage({ host, onRequestClose }: { host: Host } & PageRenderProps) {
  const tour = useTour(host)
  const { tour: loaded, records, error, open, setOpen, active, framed } = tour
  const removeButton = <RemoveLearnIii host={host} onClose={onRequestClose} />

  const caption =
    tour.openStep && loaded ? `${loaded.steps.indexOf(tour.openStep) + 1} · ${tour.openStep.title}` : undefined
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
        <div className="learn-loading" role="status" aria-busy="true" aria-label="Loading the tour">
          <Skeleton className="learn-skeleton learn-skeleton--bar" />
          <Skeleton className="learn-skeleton" />
          <Skeleton className="learn-skeleton" />
          <Skeleton className="learn-skeleton" />
        </div>
      </Frame>
    )
  }

  const total = loaded.steps.length
  const done = loaded.steps.filter((step) => records[step.id]?.status === 'complete').length
  const activeIndex = active ? loaded.steps.indexOf(active) : -1

  return (
    <Frame title={loaded.title} description={loaded.description} onClose={onRequestClose} footer={removeButton}>
      <header className="learn-head">
        <p className="learn-head-count" role="status">
          {done === total ? (
            'All steps done'
          ) : (
            <>
              Step <span className="learn-num">{activeIndex + 1}</span> of <span className="learn-num">{total}</span>
            </>
          )}
          <span className="learn-head-done">
            {' · '}
            <span className="learn-num">{done}</span> done
          </span>
        </p>
        <Button variant="ghost" size="sm" className="learn-ghost" onClick={tour.reset} disabled={done === 0}>
          Restart
        </Button>
      </header>

      <ol className="learn-steps">
        {loaded.steps.map((step, index) => {
          const record = records[step.id]
          const state = tour.stateOf(step)
          // A step opens once it is reached. Reading ahead would give away a
          // box the operator has not been shown yet.
          const reached = state !== 'pending'
          const isOpen = reached && open === step.id
          const last = index === total - 1
          const indicator = (
            <span className="learn-indicator" data-state={state} aria-hidden="true">
              <span className="learn-indicator-num">{index + 1}</span>
              <Check className="learn-indicator-check" strokeWidth={2.5} />
              {state === 'active' ? <span className={`learn-indicator-pulse ${uiClasses.pulse}`} /> : null}
            </span>
          )
          return (
            <li
              key={step.id}
              className="learn-step"
              data-state={state}
              data-open={isOpen || undefined}
              aria-current={state === 'active' ? 'step' : undefined}
            >
              {last ? null : (
                <span className="learn-line" data-done={state === 'complete' || undefined} aria-hidden="true" />
              )}
              {reached ? (
                <button
                  type="button"
                  className="learn-row"
                  aria-expanded={isOpen}
                  onClick={() => setOpen(isOpen ? null : step.id)}
                >
                  {indicator}
                  <span className="learn-title">{step.title}</span>
                  <StateChip state={state} step={step} record={record} />
                  <ChevronDown className={`learn-chevron ${uiClasses.icon}`} aria-hidden="true" />
                </button>
              ) : (
                <div className="learn-row">
                  {indicator}
                  <span className="learn-title">{step.title}</span>
                  <span className="learn-sr">not reached yet</span>
                </div>
              )}
              {reached ? (
                // Mounted while the step is reached and folded with a grid
                // track, so opening is a transition that can reverse midway
                // rather than an entrance that restarts from nothing.
                <div className="learn-panel" data-open={isOpen || undefined} inert={!isOpen}>
                  <div className="learn-panel-clip">
                    <div className="learn-panel-body learn-card">
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
export function RemoveLearnIii({
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
    else await host.iii.trigger('console::workspace::close', { screen: 'ext:learn-iii' }).catch(() => undefined)
  }, [host, onClose])
  return (
    <>
      {error ? <StatusPanel variant="alert" headline={error} role="alert" /> : null}
      <Button variant="ghost" size="sm" className="learn-ghost" onClick={remove} disabled={removing}>
        {removing ? 'Removing…' : label}
      </Button>
    </>
  )
}

function Frame({
  title = 'learn-iii',
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
        <div className="learn-scroll">
          {/* Centred column: the pane is often narrow beside a chat, but a
              page wide enough to be a whole tab should not leave the list
              stranded on one edge. */}
          <div className="learn-column">{children}</div>
        </div>
        {footer ? <div className="learn-footer">{footer}</div> : null}
      </PageMain>
    </PageShell>
  )
}
