import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { bindCondition, type Condition, type Fired, reconcileCondition } from './conditions'

/**
 * Everything the tour knows and does, shared by its two faces: the floating
 * card (the overlay slot) and the page an older console opens in a pane.
 * The engine owns progress; this keeps the step being read, the conditions
 * bound, and which console screens the open step is waiting on.
 */

/** How long the box stays on what a step's `on_closed` note points at. */
const HINT_SPOTLIGHT_MS = 5_000

export interface Step {
  id: string
  title: string
  body: string
  anchors?: string[]
  condition?: Condition
  /**
   * A prompt the step sends: into a new chat under `agent` (a profile id),
   * the way a setup example prompt starts one, or into the chat the
   * operator is in.
   */
  ask?: { text: string; label: string; agent?: string }
  /** A screen the step suggests closing, with what to show once it is gone. */
  on_closed?: { screen: string; body: string; anchors?: string[] }
  /** A screen the step opens (`label` is its button), with where it opens by hand. */
  to_open?: { screen: string; label: string; body: string; anchors: string[]; elsewhere?: string }
}

export interface Tour {
  id: string
  title: string
  description: string
  /** Step 0: the card asks before it teaches. Closed like a step, under `records.intro`. */
  intro: { id: 'intro'; title: string; body: string }
  steps: Step[]
}

export interface StepRecord {
  status: 'complete'
  at: number
  fired?: { trigger_type: string | null; payload: unknown; at: number } | null
}

export type StepRecords = Record<string, StepRecord | undefined>

interface ProgressResponse {
  tours: Record<string, { steps?: StepRecords } | undefined>
  next_tour_id: string | null
  /** When the operator signed up for product updates; never the address. */
  subscribed_at?: number | null
}

export type StepState = 'complete' | 'active' | 'pending'

export interface TourController {
  tour: Tour | null
  records: StepRecords
  error: string | null
  /** Signed up for product updates, now or on an earlier visit. */
  subscribed: boolean
  markSubscribed: () => void
  /** The step being read; the front unless the operator stepped back. */
  open: string | null
  setOpen: (stepId: string | null) => void
  openStep: Step | null
  /** The first step still to do. */
  active: Step | undefined
  stateOf: (step: Step) => StepState
  complete: (stepId: string, fired?: Fired) => void
  ask: (step: Step) => void
  /** Open the step's `to_open` screen beside the chat. */
  openScreen: (step: Step) => void
  reset: () => void
  /** The open step's `on_closed` screen has been closed. */
  closedNote: boolean
  /** The open step's `to_open` screen is not open yet, here or anywhere. */
  awaitingScreen: boolean
  /** The open step's `to_open` screen is open, but in a tab not showing. */
  screenElsewhere: boolean
  /** What the spotlight frames for the open step. */
  framed: readonly string[] | null
  /** Read progress again — the card coming up after a step closed elsewhere. */
  refresh: () => void
}

export function useTour(host: Host): TourController {
  const [tour, setTour] = useState<Tour | null>(null)
  const [records, setRecords] = useState<StepRecords>({})
  const [open, setOpen] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [subscribed, setSubscribed] = useState(false)
  // The records as last written, for the callbacks: two completions landing
  // together each see the other, and a step closes once.
  const recordsRef = useRef(records)
  recordsRef.current = records
  const completing = useRef(new Set<string>())

  useEffect(() => {
    let live = true
    void (async () => {
      try {
        const [list, progress] = await Promise.all([
          host.iii.trigger<{ tours: { id: string }[] }>('learn-iii::tours::list'),
          host.iii.trigger<ProgressResponse>('learn-iii::progress::get', {}),
        ])
        const id = progress.next_tour_id ?? list.tours[0]?.id
        if (!id) {
          if (live) setError('This engine has no tours registered.')
          return
        }
        const { tour: loaded } = await host.iii.trigger<{ tour: Tour }>('learn-iii::tours::get', { id })
        if (!live) return
        const stored = progress.tours[id]?.steps ?? {}
        setTour(loaded)
        setRecords(stored)
        setSubscribed(Boolean(progress.subscribed_at))
        setOpen(firstIncomplete(loaded, stored)?.id ?? loaded.steps[0]?.id ?? null)
      } catch (cause) {
        if (live) setError(errorMessage(cause))
      }
    })()
    return () => {
      live = false
    }
  }, [host])

  /**
   * Take in records that just closed steps, and move the reader on only from
   * the step that closed under them: someone reading an earlier step keeps
   * reading it, and finds the closed one done when they come back to it.
   */
  const settle = useCallback(
    (next: StepRecords, closed: readonly string[]) => {
      if (!tour) return
      recordsRef.current = next
      setRecords(next)
      setOpen((reading) =>
        reading !== null && closed.includes(reading) ? (firstIncomplete(tour, next)?.id ?? null) : reading,
      )
    },
    [tour],
  )

  const complete = useCallback(
    (stepId: string, fired?: Fired) => {
      if (!tour) return
      // The same step can be reported twice — its trigger and a reconcile
      // read, a delivery the engine retries — and closes once.
      if (recordsRef.current[stepId]?.status === 'complete' || completing.current.has(stepId)) return
      completing.current.add(stepId)
      // The engine owns progress, so the tour moves only once the write
      // lands. A rejected write leaves the step where it was, instead of
      // showing complete until the next reload disagrees.
      void host.iii
        .trigger<{ step?: StepRecord }>('learn-iii::steps::complete', {
          tour_id: tour.id,
          step_id: stepId,
          ...(fired ? { fired: { trigger_type: fired.trigger_type, payload: fired.payload } } : {}),
        })
        .then((result) => {
          const current = recordsRef.current
          if (current[stepId]?.status === 'complete') return
          // The worker's own record, not a second copy of it: its payload is
          // capped and its timestamps are the ones a reload will show.
          settle({ ...current, [stepId]: result.step ?? { status: 'complete' as const, at: Date.now() } }, [stepId])
        })
        .catch((cause: unknown) => setError(errorMessage(cause)))
        .finally(() => completing.current.delete(stepId))
    },
    [host, tour, settle],
  )

  const refresh = useCallback(() => {
    if (!tour) return
    void host.iii
      .trigger<ProgressResponse>('learn-iii::progress::get', {})
      .then((progress) => {
        const stored = progress.tours[tour.id]?.steps ?? {}
        const current = recordsRef.current
        const closed = tour.steps
          .filter((step) => stored[step.id]?.status === 'complete' && current[step.id]?.status !== 'complete')
          .map((step) => step.id)
        if (closed.length > 0) settle({ ...current, ...stored }, closed)
      })
      // A failed read leaves the tour as it was; the next show reads again.
      .catch(() => {})
  }, [host, tour, settle])

  /**
   * Send the step's prompt. With an `agent` the console starts a new chat
   * under that profile and sends it there, the way the setup wizard's
   * example prompts start one (`host.chat.startChat`); without one the
   * prompt goes to the chat the operator is in. The console does the
   * sending either way, so the message is a normal turn — the same one the
   * step's condition is waiting for.
   *
   * A step with a condition closes when its trigger fires; one without closes
   * here, because sending WAS the step.
   */
  const ask = useCallback(
    (step: Step) => {
      if (!step.ask) return
      const { text, agent } = step.ask
      if (agent && host.chat?.startChat) {
        void host.chat.startChat({ text, agent }).catch((cause: unknown) => setError(errorMessage(cause)))
      } else if (host.chat?.compose) {
        host.chat.compose({ text, submit: true })
      } else {
        setError('This console is too old to send a prompt for you. Type it in the chat instead.')
        return
      }
      if (!step.condition) complete(step.id)
    },
    [complete, host],
  )

  /**
   * Open the step's screen beside the chat, in the tab showing. The layout
   * change rings `console::workspace::changed`, which `useScreenPresent`
   * follows, so the step moves on by itself.
   */
  const openScreen = useCallback(
    (step: Step) => {
      if (!step.to_open) return
      void host.iii
        .trigger('console::workspace::open', { screen: step.to_open.screen, relative_to: 'chat' })
        .catch((cause: unknown) => setError(errorMessage(cause)))
    },
    [host],
  )

  // Every step that is still open for business gets its condition bound, so a
  // step can be satisfied before the operator reads down to it — and whether
  // the card is up, folded or closed, since the hook outlives the card. Keyed
  // on WHICH steps are open, not on the records object, so one step closing
  // does not unbind and rebind every other.
  const waiting = tour
    ? tour.steps
        .filter((step) => step.condition && records[step.id]?.status !== 'complete')
        .map((step) => step.id)
        .join(',')
    : ''
  useEffect(() => {
    if (!tour || !waiting) return
    const steps = tour.steps.filter((step) => waiting.split(',').includes(step.id))
    const unbinds = steps.map((step) =>
      bindCondition(host, `${tour.id}::${step.id}`, step.condition as Condition, (fired) => complete(step.id, fired)),
    )
    // Bind first, then read what each condition waits on: a write that
    // landed while nothing was listening (a reload, a restart, a console
    // without the tour loaded) still closes its step.
    let live = true
    for (const step of steps) {
      void reconcileCondition(host, step.condition as Condition).then((fired) => {
        if (live && fired) complete(step.id, fired)
      })
    }
    return () => {
      live = false
      for (const unbind of unbinds) unbind()
    }
  }, [host, tour, waiting, complete])

  const openStep = useMemo(() => tour?.steps.find((step) => step.id === open) ?? null, [tour, open])
  const active = tour ? firstIncomplete(tour, records) : undefined
  const stateOf = useCallback(
    (step: Step): StepState =>
      records[step.id]?.status === 'complete' ? 'complete' : step.id === active?.id ? 'active' : 'pending',
    [records, active],
  )

  const watching = openStep?.on_closed?.screen ?? openStep?.to_open?.screen ?? null
  const presence = useScreenPresent(host, watching)
  const closedNote = Boolean(openStep?.on_closed) && presence?.present === false
  const awaitingScreen = Boolean(openStep?.to_open) && presence?.present === false
  // Open, but only in tabs that are not the one showing: the step's anchor
  // has nothing on screen to land on, so the step points at the tab instead.
  const screenElsewhere = Boolean(openStep?.to_open) && presence?.present === true && !presence.inActiveTab
  const elsewhereTab = screenElsewhere ? presence?.tabIds[0] : undefined
  const elsewhereAnchors = useMemo(
    () => (elsewhereTab ? [`[role="tab"][data-tab-id="${elsewhereTab}"]`] : null),
    [elsewhereTab],
  )

  /**
   * A step whose `on_closed` screen is gone points at what brings it back
   * instead of at the empty space where it used to be — for a few seconds,
   * and then the box goes back to the step. That hint answers one question
   * ("where did the panel go?"), and a box still lit long after it is
   * answered is just something on the screen the operator cannot turn off.
   *
   * A `to_open` step points at where its screen opens for as long as it is
   * not open: that IS the step, so the box stays until the operator acts.
   */
  const [hintDone, setHintDone] = useState(false)
  useEffect(() => {
    if (!closedNote) {
      setHintDone(false)
      return
    }
    setHintDone(false)
    const timer = setTimeout(() => setHintDone(true), HINT_SPOTLIGHT_MS)
    return () => clearTimeout(timer)
  }, [closedNote])
  const framed =
    closedNote && !hintDone
      ? (openStep?.on_closed?.anchors ?? null)
      : awaitingScreen
        ? (openStep?.to_open?.anchors ?? null)
        : screenElsewhere
          ? elsewhereAnchors
          : (openStep?.anchors ?? null)

  const reset = useCallback(() => {
    if (!tour) return
    // Same order as `complete`: the stored progress is the truth, so the list
    // empties after the write, not before it. The keys the agent's steps
    // report into go first: left in place, the reconcile read would close
    // those steps again the moment the tour restarts.
    const reports = tour.steps.flatMap((step) =>
      step.condition?.type === 'state' ? [step.condition.config as { scope?: string; key?: string }] : [],
    )
    void Promise.all(
      reports.map(({ scope, key }) =>
        scope && key ? host.iii.trigger('state::delete', { scope, key }).catch(() => undefined) : undefined,
      ),
    )
      .then(() => host.iii.trigger('learn-iii::steps::reset', { tour_id: tour.id }))
      .then(() => {
        recordsRef.current = {}
        setRecords({})
        setOpen(tour.steps[0]?.id ?? null)
      })
      .catch((cause: unknown) => setError(errorMessage(cause)))
  }, [host, tour])

  const markSubscribed = useCallback(() => setSubscribed(true), [])

  return {
    tour,
    records,
    error,
    subscribed,
    markSubscribed,
    open,
    setOpen,
    openStep,
    active,
    stateOf,
    complete,
    ask,
    openScreen,
    reset,
    closedNote,
    awaitingScreen,
    screenElsewhere,
    framed,
    refresh,
  }
}

interface ScreenPresence {
  /** In any workspace tab. */
  present: boolean
  /** In the tab that is showing. */
  inActiveTab: boolean
  /** The tabs holding it that are not showing. */
  tabIds: string[]
}

/**
 * Whether `screen` is in any workspace tab: `null` while unknown — not
 * watching, not read yet, or a console too old to say.
 *
 * The console rings `console::workspace::changed` after every layout write,
 * opening or closing a pane or a tab included. That makes this reactive
 * rather than polled: one read of the layout per layout change, and none
 * while nothing moves. A console without the trigger leaves the binding
 * pending, and only the first read runs.
 *
 * Read once when the step opens too: the operator may have opened or closed
 * the screen before reading this far.
 */
function useScreenPresent(host: Host, screen: string | null): ScreenPresence | null {
  const [seen, setSeen] = useState<({ screen: string } & ScreenPresence) | null>(null)
  useEffect(() => {
    if (!screen) return
    let live = true
    const check = () =>
      void host.iii
        .trigger<{ tabs: { id: string; screens: string[]; active?: boolean }[] }>('console::workspace::list')
        .then((ws) => {
          if (!live) return
          const holding = ws.tabs.filter((tab) => tab.screens.includes(screen))
          // A console that does not mark the showing tab is taken to be
          // showing it: the step then frames its own anchor, as it used to.
          const inActiveTab = holding.some((tab) => tab.active !== false)
          setSeen({
            screen,
            present: holding.length > 0,
            inActiveTab,
            tabIds: holding.filter((tab) => !tab.active).map((tab) => tab.id),
          })
        })
        // A console too old for `workspace::list` just never answers; the
        // step treats unknown as "on screen", so it is never a dead end.
        .catch(() => {})
    const localId = `learn-iii::layout::${screen}`
    let offHandler: () => void = () => {}
    let offTrigger: () => void = () => {}
    try {
      offHandler = host.iii.on(localId, check)
      offTrigger = host.iii.registerTrigger({
        type: 'console::workspace::changed',
        function_id: `${localId}::${host.iii.browserId}`,
        config: {},
      })
    } catch {
      // Same rule as `bindCondition`: no handler without its trigger. The
      // read below still runs once.
      offHandler()
      offHandler = () => {}
    }
    // Bind first, then read: a change that landed before the binding still counts.
    check()
    return () => {
      live = false
      offTrigger()
      offHandler()
    }
  }, [host, screen])
  return screen && seen?.screen === screen ? seen : null
}

export function firstIncomplete(tour: Tour, records: StepRecords): Step | undefined {
  return tour.steps.find((step) => records[step.id]?.status !== 'complete')
}
