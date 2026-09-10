import { type Host, PageBody, PageHeader, PageMain, type PageRenderProps, PageShell } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { bindCondition, type Condition, type Fired } from './conditions'
import { disposeSpotlight, hideSpotlight, showSpotlight } from './spotlight'

/**
 * One list, one open step. Rows are hand-rolled elements over the console's
 * design tokens rather than shared card components: the tour has to render on
 * whatever console build is in front of the operator, including ones older
 * than the component it would otherwise import.
 */

interface Step {
  id: string
  title: string
  body: string
  anchors?: string[]
  condition?: Condition
}

interface Tour {
  id: string
  title: string
  description: string
  steps: Step[]
}

interface StepRecord {
  status: 'complete'
  at: number
  fired?: { trigger_type: string | null; payload: unknown; at: number } | null
}

type StepRecords = Record<string, StepRecord | undefined>

interface ProgressResponse {
  tours: Record<string, { steps?: StepRecords } | undefined>
  next_tour_id: string | null
}

type StepState = 'complete' | 'active' | 'pending'

export function TourPage({ host }: { host: Host } & PageRenderProps) {
  const [tour, setTour] = useState<Tour | null>(null)
  const [records, setRecords] = useState<StepRecords>({})
  const [open, setOpen] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let live = true
    void (async () => {
      try {
        const [list, progress] = await Promise.all([
          host.iii.trigger<{ tours: { id: string }[] }>('onboarding::tours::list'),
          host.iii.trigger<ProgressResponse>('onboarding::progress::get', {}),
        ])
        const id = progress.next_tour_id ?? list.tours[0]?.id
        if (!id) {
          if (live) setError('This engine has no tours registered.')
          return
        }
        const { tour: loaded } = await host.iii.trigger<{ tour: Tour }>('onboarding::tours::get', { id })
        if (!live) return
        const stored = progress.tours[id]?.steps ?? {}
        setTour(loaded)
        setRecords(stored)
        setOpen(firstIncomplete(loaded, stored)?.id ?? loaded.steps[0]?.id ?? null)
      } catch (cause) {
        if (live) setError(cause instanceof Error ? cause.message : String(cause))
      }
    })()
    return () => {
      live = false
    }
  }, [host])

  const complete = useCallback(
    (stepId: string, fired?: Fired) => {
      if (!tour) return
      setRecords((current) =>
        current[stepId]?.status === 'complete'
          ? current
          : {
              ...current,
              [stepId]: {
                status: 'complete',
                at: Date.now(),
                fired: fired ? { trigger_type: fired.trigger_type, payload: fired.payload, at: fired.at } : null,
              },
            },
      )
      // Finishing a step opens the next one that is still open for business,
      // so the list reads as one moving front instead of a closed accordion.
      setOpen(tour.steps.find((step) => step.id !== stepId && records[step.id]?.status !== 'complete')?.id ?? null)
      void host.iii
        .trigger('onboarding::steps::complete', {
          tour_id: tour.id,
          step_id: stepId,
          ...(fired ? { fired: { trigger_type: fired.trigger_type, payload: fired.payload } } : {}),
        })
        .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
    },
    [host, records, tour],
  )

  // Every step that is still open for business gets its condition bound, so a
  // step can be satisfied before the operator reads down to it.
  useEffect(() => {
    if (!tour) return
    const unbinds = tour.steps
      .filter((step) => step.condition && records[step.id]?.status !== 'complete')
      .map((step) =>
        bindCondition(host, `${tour.id}::${step.id}`, step.condition as Condition, (fired) => complete(step.id, fired)),
      )
    return () => {
      for (const unbind of unbinds) unbind()
    }
  }, [host, tour, records, complete])

  const openStep = useMemo(() => tour?.steps.find((step) => step.id === open) ?? null, [tour, open])

  // The box frames whatever step is open, and leaves with the page.
  useEffect(() => {
    showSpotlight(openStep?.anchors ?? null)
    return hideSpotlight
  }, [openStep])
  useEffect(() => disposeSpotlight, [])

  const reset = useCallback(() => {
    if (!tour) return
    setRecords({})
    setOpen(tour.steps[0]?.id ?? null)
    void host.iii.trigger('onboarding::steps::reset', { tour_id: tour.id })
  }, [host, tour])

  if (error) {
    return (
      <Frame>
        <p className="ob-error">{error}</p>
      </Frame>
    )
  }
  if (!tour) {
    return (
      <Frame>
        <p className="ob-muted">Loading the tour…</p>
      </Frame>
    )
  }

  const active = firstIncomplete(tour, records)
  const done = tour.steps.filter((step) => records[step.id]?.status === 'complete').length

  return (
    <Frame title={tour.title} description={tour.description}>
      <div className="ob-summary">
        <div
          className="ob-bar"
          role="progressbar"
          aria-valuenow={done}
          aria-valuemin={0}
          aria-valuemax={tour.steps.length}
        >
          <span style={{ width: `${(done / tour.steps.length) * 100}%` }} />
        </div>
        <span className="ob-count">
          {done} of {tour.steps.length} done
        </span>
        <button type="button" className="ob-link" onClick={reset} disabled={done === 0}>
          Restart
        </button>
      </div>

      <ol className="ob-steps">
        {tour.steps.map((step, index) => {
          const record = records[step.id]
          const state: StepState =
            record?.status === 'complete' ? 'complete' : step.id === active?.id ? 'active' : 'pending'
          // A step opens once it is reached. Reading ahead would give away a
          // box the operator has not been shown yet.
          const locked = state === 'pending'
          const isOpen = open === step.id
          return (
            <li key={step.id} className="ob-step" data-state={state} data-open={isOpen || undefined}>
              <button
                type="button"
                className="ob-step-head"
                aria-expanded={isOpen}
                disabled={locked}
                onClick={() => setOpen(isOpen ? null : step.id)}
              >
                <span className="ob-dot" data-state={state} aria-hidden="true" />
                <span className="ob-step-index">{index + 1}</span>
                <span className="ob-step-title">{step.title}</span>
                <span className="ob-step-state">{stateLabel(state, step)}</span>
              </button>
              {isOpen ? (
                <div className="ob-step-body">
                  <p className="ob-copy">{step.body}</p>
                  {step.condition ? <ConditionBlock condition={step.condition} fired={record?.fired ?? null} /> : null}
                  {state !== 'complete' && !step.condition ? (
                    <button type="button" className="ob-button" onClick={() => complete(step.id)}>
                      Got it
                    </button>
                  ) : null}
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
 * The condition, before and after. Waiting names the trigger type and the
 * binding config; fired shows the trigger and the payload the engine
 * delivered, the way the harness shows a function's own request and response.
 */
function ConditionBlock({ condition, fired }: { condition: Condition; fired: StepRecord['fired'] }) {
  const hasConfig = Object.keys(condition.config ?? {}).length > 0
  if (fired) {
    return (
      <div className="ob-trigger" data-fired="true">
        <div className="ob-trigger-head">
          <span className="ob-chip" data-tone="ok">
            trigger fired
          </span>
          <code className="ob-code">{fired.trigger_type ?? condition.type}</code>
          <span className="ob-muted">{when(fired.at)}</span>
        </div>
        <pre className="ob-payload">{format(fired.payload)}</pre>
      </div>
    )
  }
  return (
    <div className="ob-trigger">
      <div className="ob-trigger-head">
        <span className="ob-chip" data-tone="wait">
          <span className="ob-pulse" aria-hidden="true" />
          waiting
        </span>
        <code className="ob-code">{condition.type}</code>
        {hasConfig ? <code className="ob-code">{JSON.stringify(condition.config)}</code> : null}
      </div>
      <p className="ob-muted">{condition.label}</p>
      {condition.hint ? <pre className="ob-hint">{condition.hint}</pre> : null}
    </div>
  )
}

function Frame({
  title = 'Tour',
  description,
  children,
}: {
  title?: string
  description?: string
  children: React.ReactNode
}) {
  return (
    <PageShell>
      <PageMain>
        <PageHeader title={title} description={description} />
        <PageBody>
          <div className="ob-page">{children}</div>
        </PageBody>
      </PageMain>
    </PageShell>
  )
}

function firstIncomplete(tour: Tour, records: StepRecords): Step | undefined {
  return tour.steps.find((step) => records[step.id]?.status !== 'complete')
}

function stateLabel(state: StepState, step: Step): string {
  if (state === 'complete') return 'complete'
  if (state === 'pending') return 'not reached'
  return step.condition ? 'waiting' : 'in progress'
}

function when(at: number): string {
  return new Date(at).toLocaleTimeString()
}

function format(payload: unknown): string {
  try {
    return JSON.stringify(payload, null, 2) ?? 'null'
  } catch {
    return String(payload)
  }
}
