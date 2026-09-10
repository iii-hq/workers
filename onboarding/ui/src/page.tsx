import {
  Button,
  type Host,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  uiClasses,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { bindCondition, type Condition, type Fired } from './conditions'
import { disposeSpotlight, hideSpotlight, showSpotlight } from './spotlight'

/**
 * One list, one open step.
 *
 * Styling is the console's own: its utility classes and the `uiClasses`
 * recipes (`card`, `listItem`, `chip`), so the page inherits the house
 * spacing, edges and hover states. The stylesheet next door carries only what
 * those cannot express — the spotlight box, the status dot, the progress bar,
 * and the caret.
 *
 * The MARKUP is hand-rolled rather than built from shared components: the page
 * has to render on whatever console build is in front of the operator,
 * including ones older than a component it would otherwise import.
 */

/** Console class recipes, with a literal fallback for an older build that
    does not publish them. */
const ui = uiClasses ?? {
  card: 'iii-ui-card',
  listItem: 'iii-ui-list-item',
  chip: 'iii-ui-chip',
}

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

export function OnboardingPage({ host }: { host: Host } & PageRenderProps) {
  const [tour, setTour] = useState<Tour | null>(null)
  const [records, setRecords] = useState<StepRecords>({})
  const [open, setOpen] = useState<string | null>(null)
  // Which step's condition row is expanded. Independent of the step rows: a
  // fired trigger stays readable while the operator reads on.
  const [openSub, setOpenSub] = useState<string | null>(null)
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
      // A trigger that just fired opens its own row, so the operator sees the
      // payload arrive rather than having to hunt for it.
      if (fired) setOpenSub(stepId)
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
        <p className="m-0 text-base text-alert">{error}</p>
      </Frame>
    )
  }
  if (!tour) {
    return (
      <Frame>
        <p className="m-0 text-base text-ink-faint">Loading onboarding…</p>
      </Frame>
    )
  }

  const active = firstIncomplete(tour, records)
  const done = tour.steps.filter((step) => records[step.id]?.status === 'complete').length

  return (
    <Frame title={tour.title} description={tour.description}>
      <div className="flex items-center gap-4">
        <div
          className="ob-bar"
          role="progressbar"
          aria-valuenow={done}
          aria-valuemin={0}
          aria-valuemax={tour.steps.length}
        >
          <span style={{ width: `${(done / tour.steps.length) * 100}%` }} />
        </div>
        <span className="shrink-0 text-base text-ink-faint tabular-nums">
          {done} of {tour.steps.length} done
        </span>
        <Button variant="ghost" size="sm" onClick={reset} disabled={done === 0}>
          Restart
        </Button>
      </div>

      <ol className="ob-steps m-0 flex list-none flex-col gap-2 p-0">
        {tour.steps.map((step, index) => {
          const record = records[step.id]
          const state: StepState =
            record?.status === 'complete' ? 'complete' : step.id === active?.id ? 'active' : 'pending'
          // A step opens once it is reached. Reading ahead would give away a
          // box the operator has not been shown yet.
          const locked = state === 'pending'
          const isOpen = open === step.id
          return (
            <li key={step.id} className={`ob-step ${ui.card}`} data-state={state} data-open={isOpen || undefined}>
              <button
                type="button"
                className={`${ui.listItem} px-4 py-3 hover:bg-surface-hover`}
                aria-expanded={isOpen}
                disabled={locked}
                onClick={() => setOpen(isOpen ? null : step.id)}
              >
                <span className="ob-dot shrink-0" data-state={state} aria-hidden="true" />
                <span className="shrink-0 font-mono text-base text-ink-faint tabular-nums">{index + 1}</span>
                <span className="min-w-0 flex-1 text-lg font-medium">{step.title}</span>
                <span className="shrink-0 text-sm text-ink-faint">{stateLabel(state, step)}</span>
              </button>
              {isOpen ? (
                <div className="ob-open flex flex-col gap-3 px-4 pb-4 pl-11">
                  <p className="m-0 text-base leading-relaxed text-ink text-pretty">{step.body}</p>
                  {state !== 'complete' && !step.condition ? (
                    <Button className="self-start" onClick={() => complete(step.id)}>
                      Got it
                    </Button>
                  ) : null}
                </div>
              ) : null}
              {/* A step's trigger appears once the step is reached, and stays
                  after it is done. Ahead of the front it is hidden: the row
                  would give away what the step is about to ask for. */}
              {step.condition && state !== 'pending' ? (
                <ol className="m-0 list-none px-4 pb-4 pl-11">
                  <ConditionRow
                    condition={step.condition}
                    fired={record?.fired ?? null}
                    open={openSub === step.id}
                    onToggle={() => setOpenSub(openSub === step.id ? null : step.id)}
                  />
                </ol>
              ) : null}
            </li>
          )
        })}
      </ol>
    </Frame>
  )
}

/**
 * A step's condition as its own row under the step: the trigger, its state,
 * and — once it fires — the payload the engine delivered, the way the harness
 * shows a function call. Collapsed until the operator wants the detail.
 */
function ConditionRow({
  condition,
  fired,
  open,
  onToggle,
}: {
  condition: Condition
  fired: StepRecord['fired']
  open: boolean
  onToggle: () => void
}) {
  const hasConfig = Object.keys(condition.config ?? {}).length > 0
  return (
    <li className={`ob-sub ${ui.card}`} data-state={fired ? 'fired' : 'waiting'}>
      <button
        type="button"
        className={`${ui.listItem} px-3 py-2 hover:bg-surface-hover`}
        aria-expanded={open}
        onClick={onToggle}
      >
        <span className="ob-caret shrink-0 text-ink-faint" data-open={open || undefined} aria-hidden="true">
          ›
        </span>
        <span className={`${ui.chip} shrink-0 ${fired ? 'bg-ok-muted text-ok' : 'bg-accent-muted text-accent'}`}>
          {fired ? 'trigger fired' : <span className="ob-pulse" aria-hidden="true" />}
          {fired ? null : 'waiting'}
        </span>
        <code className="shrink-0 rounded-sm bg-panel px-2 py-1 font-mono text-sm">
          {fired?.trigger_type ?? condition.type}
        </code>
        <span className="min-w-0 flex-1 truncate text-sm text-ink-faint">
          {fired ? when(fired.at) : condition.label}
        </span>
      </button>
      {open ? (
        <div className="ob-open flex flex-col gap-2 px-3 pb-3 pl-8">
          {hasConfig ? (
            <p className="m-0 text-sm text-ink-faint">
              binding{' '}
              <code className="rounded-sm bg-panel px-2 py-1 font-mono text-sm">
                {JSON.stringify(condition.config)}
              </code>
            </p>
          ) : null}
          {fired ? (
            <pre className="ob-pre">{format(fired.payload)}</pre>
          ) : (
            <>
              <p className="m-0 text-base text-ink-faint">{condition.label}</p>
              {condition.hint ? <pre className="ob-pre select-all text-ink">{condition.hint}</pre> : null}
            </>
          )}
        </div>
      ) : null}
    </li>
  )
}

function Frame({
  title = 'onboarding',
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
          {/* Centred column: the pane is often narrow beside a chat, but a
              page wide enough to be a whole tab should not leave the list
              stranded on one edge. */}
          <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-6">{children}</div>
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
