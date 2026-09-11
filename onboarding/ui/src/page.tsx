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
  /** A console screen the step's button opens beside the tour, e.g. `traces`. */
  screen?: string
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
  // Which step's condition row is expanded. Opened by the operator only — a
  // trigger that fires does not throw its payload open over what is being
  // read; the row is there when the detail is wanted.
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
      // The engine owns progress, so the list moves only once the write
      // lands. A rejected write leaves the step where it was, instead of
      // showing complete until the next reload disagrees.
      void host.iii
        .trigger('onboarding::steps::complete', {
          tour_id: tour.id,
          step_id: stepId,
          ...(fired ? { fired: { trigger_type: fired.trigger_type, payload: fired.payload } } : {}),
        })
        .then(() => {
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
          // Finishing a step opens the next one that is still open for
          // business, so the list reads as one moving front instead of a
          // closed accordion.
          setOpen(tour.steps.find((step) => step.id !== stepId && records[step.id]?.status !== 'complete')?.id ?? null)
        })
        .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
    },
    [host, records, tour],
  )

  /**
   * The step's button. A step that names a screen opens it in the workspace
   * first, to the RIGHT of this page rather than the console's default (right
   * of chat) — a panel the tour is pointing at belongs on the far side of the
   * tour, not wedged between the tour and the conversation.
   *
   * The widths ride along in the same call, so placement and sizing are one
   * write: chat, the tour, then the panel. `sizes` is rejected outright when
   * it does not match the column count, which is why it is sent only for the
   * three-column layout this page knows it is making.
   *
   * The step closes whether or not the call lands: an older console rejects
   * `relative_to`, and the step is about reading the panel, not about us.
   */
  const openScreen = useCallback(
    (step: Step) => {
      const placed = step.screen
        ? host.iii
            .trigger('console::workspace::open', {
              screen: step.screen,
              relative_to: 'ext:onboarding',
              direction: 'right',
              sizes: [0.3, 0.4, 0.3],
            })
            .catch(() => {})
        : Promise.resolve()
      void placed.then(() => complete(step.id))
    },
    [complete, host],
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
    // Same order as `complete`: the stored progress is the truth, so the list
    // empties after the write, not before it.
    void host.iii
      .trigger('onboarding::steps::reset', { tour_id: tour.id })
      .then(() => {
        setRecords({})
        setOpen(tour.steps[0]?.id ?? null)
      })
      .catch((cause: unknown) => setError(cause instanceof Error ? cause.message : String(cause)))
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
                  {step.condition?.prompt && state !== 'complete' ? (
                    <Copyable label="or ask the agent" text={step.condition.prompt} />
                  ) : null}
                  {step.id === 'stay-in-touch' ? <StayInTouch host={host} /> : null}
                  {state !== 'complete' && !step.condition ? (
                    <Button className="self-start" onClick={() => openScreen(step)}>
                      {step.screen ? `Open ${step.screen}` : 'Got it'}
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

/** The product-update links. Kept here and not in the tour content: they are
    chrome for one step, not text. */
const SOCIALS: { label: string; href: string; path: string }[] = [
  {
    label: 'Discord',
    href: 'https://discord.gg/iiidev',
    path: 'M20.317 4.37a19.79 19.79 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028 14.09 14.09 0 0 0 1.226-1.994.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.099.245.197.372.292a.077.077 0 0 1-.006.128 12.299 12.299 0 0 1-1.873.891.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.84 19.84 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.331c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z',
  },
  {
    label: 'GitHub',
    href: 'https://github.com/iii-hq/iii',
    path: 'M12 0C5.37 0 0 5.37 0 12a12 12 0 0 0 8.205 11.385c.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23a9.6 9.6 0 0 1 3-.405c1.02 0 2.04.135 3 .405 2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.005 12.005 0 0 0 24 12c0-6.63-5.37-12-12-12z',
  },
  {
    label: 'X',
    href: 'https://x.com/iiidevs',
    path: 'M18.901 1.153h3.68l-8.04 9.19L24 22.846h-7.406l-5.8-7.584-6.638 7.584H.474l8.6-9.83L0 1.154h7.594l5.243 6.932ZM17.61 20.644h2.039L6.486 3.24H4.298Z',
  },
  {
    label: 'LinkedIn',
    href: 'https://www.linkedin.com/company/iii-dev',
    path: 'M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286zM5.337 7.433a2.062 2.062 0 0 1-2.063-2.065 2.064 2.064 0 1 1 2.063 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.225 0z',
  },
]

/**
 * The signup box, then the same places as links. The worker does the POST —
 * the page only carries the address the operator typed.
 */
function StayInTouch({ host }: { host: Host }) {
  const [email, setEmail] = useState('')
  const [status, setStatus] = useState<'idle' | 'sending' | 'done' | 'failed'>('idle')
  const [message, setMessage] = useState('')

  const submit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault()
      if (!email.trim() || status === 'sending') return
      setStatus('sending')
      void host.iii
        .trigger('onboarding::subscribe', { email: email.trim(), source: 'onboarding_flow' })
        .then((result: unknown) => {
          const already = (result as { already?: boolean } | null)?.already
          setStatus('done')
          setMessage(already ? 'You are already on the list.' : 'You are on the list.')
        })
        .catch((error: unknown) => {
          setStatus('failed')
          setMessage(error instanceof Error ? error.message : 'the signup did not go through')
        })
    },
    [email, host, status],
  )

  return (
    <div className="flex flex-col gap-3">
      {status === 'done' ? (
        <p className="m-0 text-base text-ok">{message}</p>
      ) : (
        <form className="flex flex-wrap items-center gap-2" onSubmit={submit}>
          <input
            type="email"
            required
            value={email}
            onChange={(event) => setEmail(event.target.value)}
            placeholder="you@example.com"
            aria-label="Email address for product updates"
            className="ob-input min-w-0 flex-1"
          />
          <Button type="submit" disabled={status === 'sending'}>
            {status === 'sending' ? 'Sending…' : 'Keep me posted'}
          </Button>
        </form>
      )}
      {status === 'failed' ? <p className="m-0 text-sm text-alert">{message}</p> : null}
      <div className="flex items-center gap-2">
        {SOCIALS.map((social) => (
          <a
            key={social.label}
            href={social.href}
            target="_blank"
            rel="noreferrer"
            aria-label={social.label}
            title={social.label}
            className="ob-social"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" className="size-4">
              <path d={social.path} />
            </svg>
            <span className="sr-only">{social.label}</span>
          </a>
        ))}
      </div>
      <a
        href="https://iii.dev/docs"
        target="_blank"
        rel="noreferrer"
        className="self-start text-base text-accent underline underline-offset-2"
      >
        Read the docs
      </a>
    </div>
  )
}

/** A block the operator can copy in one click — the fallback is the text
    itself, which is selectable either way. */
function Copyable({ label, text }: { label: string; text: string }) {
  // The clipboard write is the event; there is no timer winding the label
  // back. It says `copied` until the text itself changes, which is the only
  // thing that makes the old label wrong.
  const [copied, setCopied] = useState<string | null>(null)
  const copy = useCallback(() => {
    void navigator.clipboard?.writeText(text).then(() => setCopied(text))
  }, [text])
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <span className="text-sm text-ink-faint">{label}</span>
        <Button variant="ghost" size="sm" onClick={copy}>
          {copied === text ? 'copied' : 'copy'}
        </Button>
      </div>
      <pre className="ob-pre select-all text-ink">{text}</pre>
    </div>
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
    <PageShell className="ob-page">
      <PageMain className="ob-page">
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
