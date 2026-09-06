import {
  Button,
  EmptyState,
  type Host,
  IconButton,
  Input,
  List,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  SegmentedControl,
  Skeleton,
  StatusPanel,
  Table,
  TableBody,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  uiClasses,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  allowScheduleCalls,
  createSchedule,
  errorMessage,
  type FunctionSummary,
  listAllSchedules,
  listFunctions,
  listSystemCronBindings,
  removeSessionCronTask,
  resolveModel,
  SCHEDULE_SESSION_METADATA,
  type SessionCronTask,
  type SystemCronBinding,
  sendToSession,
} from '../lib/api'
import { nextCronRun } from '../lib/cron'
import {
  createSchedulePrompt,
  manualSchedulePrompt,
  replaceSchedulePrompt,
} from '../lib/prompts'
import { BindingDetail, TaskDetail } from './detail'
import { ManualTaskForm, type ManualTaskSpec } from './manual-form'
import { parseCronPanelContext } from './panel-context'
import {
  BindingListRow,
  BindingRow,
  cadenceLabel,
  TaskListRow,
  TaskRow,
} from './rows'
import {
  byNextRun,
  countByStatus,
  type Filter,
  matchesBindingQuery,
  matchesFilter,
  matchesQuery,
} from './status'

type View = 'tasks' | 'bindings'
type Feedback = { tone: 'success' | 'warn' | 'alert'; message: string }
type Inspector =
  | { kind: 'new' }
  | { kind: 'task'; task: SessionCronTask }
  | { kind: 'binding'; binding: SystemCronBinding }
  | null

let mountSequence = 0

/** Bind an engine trigger to a browser-local handler, following the console's
    own convention: `on()` registers under `<fn>::<browserId>`, so the trigger
    must target that id, and the trigger is disposed before the handler. */
function subscribe<T>(
  host: Host,
  functionId: string,
  type: string,
  config: Record<string, unknown>,
  onEvent: (event: T) => void,
): () => void {
  const offHandler = host.iii.on<T>(functionId, onEvent)
  const offTrigger = host.iii.registerTrigger({
    type,
    function_id: `${functionId}::${host.iii.browserId}`,
    config,
  })
  return () => {
    try {
      offTrigger()
    } finally {
      offHandler()
    }
  }
}

function ClockIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="8.5" stroke="currentColor" strokeWidth="1.7" />
      <path
        d="M12 7.5v5l3.3 2"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

function RefreshIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M19 8a7.5 7.5 0 1 0 .2 7.7M19 4v4h-4"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

function SearchIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle
        cx="10.8"
        cy="10.8"
        r="6.3"
        stroke="currentColor"
        strokeWidth="1.7"
      />
      <path
        d="m16 16 4 4"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </svg>
  )
}

function PlusIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M12 5v14M5 12h14"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  )
}

function ChevronIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="m9 6 6 6-6 6"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

function SparkIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      className={className}
      aria-hidden="true"
      fill="currentColor"
    >
      <path d="M8 1.5l1.2 3.4 3.3 1.2-3.3 1.2L8 10.7 6.8 7.3 3.5 6.1l3.3-1.2L8 1.5zM12.5 10l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6.6-1.6z" />
    </svg>
  )
}

function CloseIcon({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="m7 7 10 10M17 7 7 17"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </svg>
  )
}

export function CronSchedulesPage({
  host,
  panelSide = 'left',
  onRequestClose,
  conversationId,
  panelContext,
  commands,
}: { host: Host } & Partial<PageRenderProps>) {
  const [view, setView] = useState<View>('tasks')
  const [tasks, setTasks] = useState<SessionCronTask[]>([])
  const [bindings, setBindings] = useState<SystemCronBinding[]>([])
  const [functions, setFunctions] = useState<FunctionSummary[]>([])
  const [tasksLoading, setTasksLoading] = useState(false)
  const [bindingsLoading, setBindingsLoading] = useState(false)
  const [tasksError, setTasksError] = useState<string | null>(null)
  const [bindingsError, setBindingsError] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [composer, setComposer] = useState('')
  const [sending, setSending] = useState(false)
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [inspector, setInspector] = useState<Inspector>(null)
  const [replacementTask, setReplacementTask] =
    useState<SessionCronTask | null>(null)
  const [statusFilter, setStatusFilter] = useState<Filter>('all')
  const [narrow, setNarrow] = useState(false)
  const [now, setNow] = useState(() => new Date())
  /** The schedule session a send is waiting on. `harness::send` resolves only
      when the whole turn is done, so following its answer would leave the
      operator on an unrelated chat for the length of a registration.
      `session::created` fires as soon as the session exists, which is both
      the moment worth following and the last moment before the turn's first
      call meets the approval gate. */
  const awaitedSession = useRef<{ title: string; since: number } | null>(null)

  const instanceId = useRef(++mountSequence)
  const tasksRequest = useRef(0)
  const bindingsRequest = useRef(0)
  const sendRequest = useRef(0)
  const layoutRef = useRef<HTMLDivElement | null>(null)
  const inspectorRef = useRef<HTMLElement | null>(null)
  const lastFocus = useRef<HTMLElement | null>(null)
  const composerRef = useRef<HTMLInputElement | null>(null)
  const searchRef = useRef<HTMLInputElement | null>(null)
  const openConversationRef = useRef<(sessionId: string) => void>(() => {})

  /** A schedule runs in a session of its own, and a session that has never
      taken a turn inherits no model — so a send has to name one. The operator's
      last pick is the only defensible source: choosing off the catalogue looks
      helpful until the guess lands on a model that cannot emit a function call
      and the whole turn is spent failing to register anything. */
  const model = resolveModel(host, conversationId)

  const requireModel = (): string | undefined => {
    if (!model) {
      setFeedback({
        tone: 'alert',
        message:
          'No model chosen yet. Pick one in a chat composer, then schedule.',
      })
      setSending(false)
    }
    return model
  }

  const openInspector = useCallback((next: Exclude<Inspector, null>) => {
    lastFocus.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null
    setInspector(next)
  }, [])

  const closeInspector = useCallback(() => {
    setInspector(null)
    const target = lastFocus.current
    lastFocus.current = null
    window.requestAnimationFrame(() => {
      if (target?.isConnected) target.focus()
    })
  }, [])

  const refreshTasks = useCallback(async () => {
    const request = ++tasksRequest.current
    setTasksLoading(true)
    try {
      const next = await listAllSchedules(host, conversationId ?? undefined)
      if (request !== tasksRequest.current) return
      setTasks(next)
      setTasksError(null)
    } catch (error) {
      if (request !== tasksRequest.current) return
      setTasksError(errorMessage(error))
    } finally {
      if (request === tasksRequest.current) setTasksLoading(false)
    }
  }, [host, conversationId])

  const refreshBindings = useCallback(async () => {
    const request = ++bindingsRequest.current
    setBindingsLoading(true)
    const [bindingsResult, functionsResult] = await Promise.allSettled([
      listSystemCronBindings(host),
      listFunctions(host),
    ])
    if (request !== bindingsRequest.current) return
    if (bindingsResult.status === 'fulfilled') {
      setBindings(bindingsResult.value)
      setBindingsError(null)
    } else {
      setBindingsError(errorMessage(bindingsResult.reason))
    }
    if (functionsResult.status === 'fulfilled')
      setFunctions(functionsResult.value)
    setBindingsLoading(false)
  }, [host])

  const refreshAll = useCallback(async () => {
    await Promise.all([refreshTasks(), refreshBindings()])
    setNow(new Date())
  }, [refreshTasks, refreshBindings])

  useEffect(() => {
    void refreshTasks()
  }, [refreshTasks])

  useEffect(() => {
    void refreshBindings()
  }, [refreshBindings])

  useEffect(() => {
    // Relative labels move at minute granularity, and a hidden tab moves
    // nothing anybody can see.
    const tick = () => {
      if (!document.hidden) setNow(new Date())
    }
    const interval = window.setInterval(tick, 60_000)
    const onVisible = () => tick()
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      window.clearInterval(interval)
      document.removeEventListener('visibilitychange', onVisible)
    }
  }, [])

  useEffect(() => {
    const node = layoutRef.current
    if (!node) return
    const update = () => setNarrow(node.getBoundingClientRect().width <= 760)
    update()
    const observer = new ResizeObserver(update)
    observer.observe(node)
    return () => observer.disconnect()
  }, [])

  const inspectorIdentity =
    inspector?.kind === 'task'
      ? `task:${inspector.task.subscriptionId}`
      : inspector?.kind === 'binding'
        ? `binding:${inspector.binding.id}`
        : (inspector?.kind ?? '')

  useEffect(() => {
    if (!narrow || !inspectorIdentity) return
    const frame = window.requestAnimationFrame(() => {
      inspectorRef.current
        ?.querySelector<HTMLElement>(
          'button, input, textarea, [tabindex]:not([tabindex="-1"])',
        )
        ?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [narrow, inspectorIdentity])

  useEffect(() => {
    setInspector((current) => {
      if (current?.kind === 'task') {
        const next = tasks.find(
          (task) => task.subscriptionId === current.task.subscriptionId,
        )
        if (next)
          return next === current.task ? current : { kind: 'task', task: next }
        return tasksLoading ? current : null
      }
      if (current?.kind === 'binding') {
        const next = bindings.find(
          (binding) => binding.id === current.binding.id,
        )
        if (next)
          return next === current.binding
            ? current
            : { kind: 'binding', binding: next }
        return bindingsLoading ? current : null
      }
      return current
    })
  }, [tasks, bindings, tasksLoading, bindingsLoading])

  useEffect(
    () =>
      subscribe<{ session_id?: string; title?: string; created_at?: number }>(
        host,
        `iii::cron_ui_session_created_${instanceId.current}`,
        'session::created',
        { metadata: SCHEDULE_SESSION_METADATA },
        (event) => {
          const awaited = awaitedSession.current
          if (!awaited || !event?.session_id) return
          if (event.title !== awaited.title) return
          if ((event.created_at ?? 0) < awaited.since) return
          awaitedSession.current = null
          void allowScheduleCalls(host, event.session_id)
          openConversationRef.current(event.session_id)
        },
      ),
    [host],
  )

  useEffect(() => {
    // A schedule can be registered from any session, so this listens to all of
    // them; the doorbell rings twice per change, hence the trailing coalesce.
    let timer = 0
    const off = subscribe(
      host,
      `iii::cron_ui_triggers_changed_${instanceId.current}`,
      'harness::triggers-changed',
      {},
      () => {
        window.clearTimeout(timer)
        timer = window.setTimeout(() => void refreshTasks(), 250)
      },
    )
    return () => {
      window.clearTimeout(timer)
      off()
    }
  }, [host, refreshTasks])

  const submitNaturalTask = async () => {
    const taskRequest = composer.trim()
    if (!taskRequest || sending) return
    const request = ++sendRequest.current
    const replacing = replacementTask
    setSending(true)
    setFeedback(null)
    try {
      const model = requireModel()
      if (!model) return
      const instruction = replacing
        ? replaceSchedulePrompt(replacing.subscriptionId, taskRequest)
        : createSchedulePrompt(taskRequest)

      if (replacing) {
        await sendToSession(host, replacing.sessionId, instruction, model)
        openConversation(replacing.sessionId)
      } else {
        const title = taskRequest.slice(0, 72)
        awaitedSession.current = { title, since: Date.now() }
        const sessionId = await createSchedule(host, {
          title,
          instruction,
          model,
        })
        // Usually the subscription got there first; this covers a console too
        // old to deliver session::created.
        awaitedSession.current = null
        openConversation(sessionId)
      }
      if (request !== sendRequest.current) return
      setComposer('')
      setReplacementTask(null)
      setFeedback({
        tone: 'success',
        message: replacing
          ? 'Safe replacement request sent. The old task is retired only after the new registration succeeds.'
          : 'Request sent to Harness. This list updates when the task is registered.',
      })
    } catch (error) {
      if (request !== sendRequest.current) return
      setFeedback({ tone: 'alert', message: errorMessage(error) })
    } finally {
      if (request === sendRequest.current) setSending(false)
    }
  }

  const sendManualTask = async (spec: ManualTaskSpec) => {
    const request = ++sendRequest.current
    setSending(true)
    setFeedback(null)
    try {
      const registration: Record<string, unknown> = {
        trigger_type: 'cron',
        config: { expression: spec.expression },
        label: spec.label,
        once: false,
      }
      if (spec.maxFires) registration.lifecycle = { max_fires: spec.maxFires }
      if (spec.delivery === 'call') {
        registration.function_id = spec.functionId
        registration.metadata = {
          payload: spec.payload,
          event_into: spec.eventInto || '/cron_event',
        }
      }
      const model = requireModel()
      if (!model) return
      awaitedSession.current = { title: spec.label, since: Date.now() }
      const sessionId = await createSchedule(host, {
        title: spec.label,
        instruction: manualSchedulePrompt(registration),
        model,
      })
      awaitedSession.current = null
      openConversation(sessionId)
      if (request !== sendRequest.current) return
      closeInspector()
      setFeedback({
        tone: 'success',
        message:
          'Registration request sent. Harness will add the task after the active turn completes the call.',
      })
    } catch (error) {
      if (request !== sendRequest.current) return
      setFeedback({ tone: 'alert', message: errorMessage(error) })
    } finally {
      if (request === sendRequest.current) setSending(false)
    }
  }

  const requestTaskChange = (task: SessionCronTask) => {
    if (task.target) {
      setFeedback({
        tone: 'warn',
        message:
          'Call tasks cannot be replaced safely because Harness does not expose their stored payload template. Remove and recreate this task with the complete call settings.',
      })
      return
    }
    setView('tasks')
    setReplacementTask(task)
    setComposer('')
    setInspector(null)
    setFeedback({
      tone: 'warn',
      message:
        'Describe the complete replacement. Harness will retire the old task only after the new registration succeeds.',
    })
    window.requestAnimationFrame(() => composerRef.current?.focus())
  }

  const removeTask = async (task: SessionCronTask) => {
    try {
      const removed = await removeSessionCronTask(
        host,
        task.sessionId,
        task.subscriptionId,
      )
      setFeedback({
        tone: removed ? 'success' : 'warn',
        message: removed
          ? 'Scheduled task removed.'
          : 'The task was already retired.',
      })
      closeInspector()
      await refreshTasks()
    } catch (error) {
      setFeedback({ tone: 'alert', message: errorMessage(error) })
    }
  }

  /** Show a schedule's own conversation. Older consoles have no such API, so
      the page degrades to leaving the workspace alone. */
  const openConversation = (sessionId: string) => {
    host.chat?.selectConversation?.(sessionId)
  }
  openConversationRef.current = openConversation

  const copyId = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value)
      setFeedback({ tone: 'success', message: 'Subscription id copied.' })
    } catch {
      setFeedback({ tone: 'warn', message: 'Could not reach the clipboard.' })
    }
  }

  const handleInspectorKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      closeInspector()
      return
    }
    if (!narrow || event.key !== 'Tab' || !inspectorRef.current) return
    const focusable = [
      ...inspectorRef.current.querySelectorAll<HTMLElement>(
        'button, input, textarea, select, [href], [tabindex]:not([tabindex="-1"])',
      ),
    ].filter(
      (element) =>
        !element.matches(':disabled') && element.getClientRects().length > 0,
    )
    if (focusable.length === 0) {
      event.preventDefault()
      return
    }
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault()
      last.focus()
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault()
      first.focus()
    }
  }

  // A context from the palette source (a schedule by subscription id) or the
  // setup-time "New schedule…" command (`{ action: 'new' }`). A schedule is
  // left unapplied until it shows up in `tasks` — the page can mount before
  // the first fetch resolves.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    const context = parseCronPanelContext(panelContext.context)
    if (!context) {
      appliedContextRef.current = panelContext.id
      return
    }
    if (context.action === 'new') {
      appliedContextRef.current = panelContext.id
      setView('tasks')
      setReplacementTask(null)
      openInspector({ kind: 'new' })
      return
    }
    const task = tasks.find((t) => t.subscriptionId === context.subscriptionId)
    if (!task) return // wait for the list to load
    appliedContextRef.current = panelContext.id
    setView('tasks')
    openInspector({ kind: 'task', task })
  }, [panelContext, tasks, openInspector])

  const counts = countByStatus(tasks, now.getTime())
  const orderedTasks = useMemo(() => {
    const runs = new Map(
      tasks.map((task) => [
        task.subscriptionId,
        nextCronRun(task.expression, now)?.getTime() ??
          Number.POSITIVE_INFINITY,
      ]),
    )
    return [...tasks]
      .filter((task) => matchesFilter(task, statusFilter, now.getTime()))
      .filter((task) =>
        matchesQuery(task, cadenceLabel(task.expression), query),
      )
      .sort(
        byNextRun(
          (task) => runs.get(task.subscriptionId) ?? Number.POSITIVE_INFINITY,
        ),
      )
  }, [tasks, statusFilter, query, now])

  const orderedBindings = useMemo(
    () => bindings.filter((binding) => matchesBindingQuery(binding, query)),
    [bindings, query],
  )

  const loading = view === 'tasks' ? tasksLoading : bindingsLoading
  const error = view === 'tasks' ? tasksError : bindingsError
  const showDetail = inspector !== null
  const detailOnly = narrow && showDetail

  // The page's verbs, for the palette and for the keyboard while this pane
  // has the focus. The keys stay clear of the console's own.
  useEffect(
    () =>
      commands?.register([
        {
          id: 'focus-composer',
          title: 'Focus the schedule composer',
          detail: 'Describe a routine to schedule in plain language',
          keywords: ['compose', 'describe'],
          enabled: () => !detailOnly,
          run: () => composerRef.current?.focus(),
        },
        {
          id: 'new-schedule',
          title: 'New schedule',
          detail: 'Open the manual schedule form',
          keywords: ['create', 'cron', 'add'],
          run: () => {
            setView('tasks')
            setReplacementTask(null)
            openInspector({ kind: 'new' })
          },
        },
        {
          id: 'search',
          title: 'Search schedules',
          detail: 'Filter schedules and system bindings by name',
          keywords: ['filter', 'find'],
          run: () => searchRef.current?.focus(),
        },
        {
          id: 'refresh',
          title: 'Refresh',
          detail: 'Reload schedules and system bindings',
          keywords: ['reload'],
          run: () => void refreshAll(),
        },
      ]),
    [commands, detailOnly, openInspector, refreshAll],
  )

  const detailBody =
    inspector === null ? null : inspector.kind === 'new' ? (
      <ManualTaskForm
        functions={functions}
        sending={sending}
        onClose={closeInspector}
        onSubmit={(spec) => void sendManualTask(spec)}
      />
    ) : inspector.kind === 'task' ? (
      <TaskDetail
        key={inspector.task.subscriptionId}
        task={inspector.task}
        now={now}
        onReplace={() => requestTaskChange(inspector.task)}
        onRemove={() => void removeTask(inspector.task)}
        onCopyId={() => void copyId(inspector.task.subscriptionId)}
        onOpenConversation={() => openConversation(inspector.task.sessionId)}
      />
    ) : (
      <BindingDetail binding={inspector.binding} now={now} />
    )

  const detailScroll = (
    <section
      className="cron-ui-detail-scroll"
      ref={inspectorRef}
      onKeyDown={handleInspectorKeyDown}
      aria-label="Schedule details"
      tabIndex={-1}
    >
      {detailBody}
    </section>
  )

  return (
    <PageShell className="cron-ui-shell">
      <PageHeader
        icon={<ClockIcon />}
        title="Cron"
        description="Schedules and system bindings."
        actions={
          <div className="cron-ui-header-actions">
            <div className="cron-ui-search">
              <SearchIcon className={uiClasses.icon} />
              <Input
                ref={searchRef}
                type="search"
                value={query}
                onChange={setQuery}
                placeholder="Search schedules"
                aria-label="Search schedules"
                autoComplete="off"
                spellCheck={false}
              />
            </div>
            <IconButton
              label="Refresh schedules"
              onClick={() => void refreshAll()}
              disabled={tasksLoading || bindingsLoading}
            >
              <RefreshIcon
                className={
                  loading ? `${uiClasses.icon} cron-ui-spin` : uiClasses.icon
                }
              />
            </IconButton>
            <Button
              variant="primary"
              size="sm"
              onClick={() => {
                setView('tasks')
                setReplacementTask(null)
                openInspector({ kind: 'new' })
              }}
            >
              <PlusIcon />
              New schedule
            </Button>
          </div>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageMain className="cron-ui-main">
          {detailOnly ? (
            <section
              className="cron-ui-detail-page"
              aria-label="Schedule details"
            >
              <div className="cron-ui-detail-bar">
                <Button variant="ghost" size="sm" onClick={closeInspector}>
                  <ChevronIcon className={`${uiClasses.icon} cron-ui-back`} />
                  All schedules
                </Button>
              </div>
              {detailScroll}
            </section>
          ) : (
            <div className="cron-ui-content" ref={layoutRef}>
              <form
                className="cron-ui-composer"
                onSubmit={(event) => {
                  event.preventDefault()
                  void submitNaturalTask()
                }}
              >
                <SparkIcon
                  className={`${uiClasses.icon} cron-ui-composer-icon`}
                />
                <Input
                  ref={composerRef}
                  data-autofocus=""
                  value={composer}
                  onChange={setComposer}
                  placeholder={
                    replacementTask
                      ? `Describe the replacement for ${replacementTask.label ?? 'this schedule'}…`
                      : 'Describe a routine to schedule…'
                  }
                  aria-label="Describe a routine to schedule"
                  disabled={sending || !model}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="sm"
                  disabled={sending || !model || composer.trim().length === 0}
                >
                  {sending ? 'Scheduling…' : 'Schedule'}
                </Button>
              </form>

              {feedback ? (
                <StatusPanel
                  variant={feedback.tone}
                  headline={feedback.message}
                />
              ) : null}

              {model ? null : (
                <StatusPanel
                  variant="info"
                  headline="Choose a model before scheduling"
                  detail="Each schedule runs in a session of its own, which needs a model named up front. Pick one in a chat composer and it is used here too."
                />
              )}

              <Tabs
                value={view}
                onValueChange={(value) => setView(value as View)}
              >
                <div className="cron-ui-toolbar">
                  <TabsList>
                    <TabsTrigger value="tasks">
                      Schedules
                      <span className="cron-ui-count">{tasks.length}</span>
                    </TabsTrigger>
                    <TabsTrigger value="bindings">
                      System bindings
                      <span className="cron-ui-count">{bindings.length}</span>
                    </TabsTrigger>
                  </TabsList>
                  {view === 'tasks' ? (
                    <SegmentedControl
                      aria-label="Filter by status"
                      value={statusFilter}
                      onChange={setStatusFilter}
                      options={[
                        {
                          value: 'all',
                          label: `All ${counts.all}`,
                          icon: false,
                        },
                        {
                          value: 'active',
                          label: `Active ${counts.active}`,
                          icon: false,
                        },
                        {
                          value: 'ending',
                          label: `Ending soon ${counts.ending}`,
                          icon: false,
                        },
                        {
                          value: 'finished',
                          label: `Finished ${counts.finished}`,
                          icon: false,
                        },
                      ]}
                    />
                  ) : null}
                </div>

                <TabsContent value="tasks" className="cron-ui-tab-content">
                  {error ? (
                    <StatusPanel
                      variant="alert"
                      headline="Could not read schedules"
                      detail={error}
                    />
                  ) : null}
                  {loading && orderedTasks.length === 0 ? (
                    <div className="cron-ui-skeletons">
                      <Skeleton className="cron-ui-skeleton" />
                      <Skeleton className="cron-ui-skeleton" />
                      <Skeleton className="cron-ui-skeleton" />
                    </div>
                  ) : orderedTasks.length === 0 ? (
                    <EmptyState
                      icon={ClockIcon}
                      title={
                        tasks.length === 0
                          ? 'No schedules yet'
                          : 'Nothing matches'
                      }
                      description={
                        tasks.length === 0
                          ? 'Describe a routine above, or write the cron expression yourself.'
                          : 'Change the filter or clear the search to see the rest.'
                      }
                      action={
                        tasks.length === 0
                          ? {
                              label: 'New schedule',
                              onClick: () => {
                                setReplacementTask(null)
                                openInspector({ kind: 'new' })
                              },
                            }
                          : undefined
                      }
                    />
                  ) : narrow ? (
                    <List>
                      {orderedTasks.map((task) => (
                        <TaskListRow
                          key={task.subscriptionId}
                          task={task}
                          now={now}
                          selected={
                            inspector?.kind === 'task' &&
                            inspector.task.subscriptionId ===
                              task.subscriptionId
                          }
                          onOpen={() => openInspector({ kind: 'task', task })}
                        />
                      ))}
                    </List>
                  ) : (
                    <TableViewport>
                      <TableFrame>
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>Status</TableHead>
                              <TableHead>Schedule</TableHead>
                              <TableHead>Target</TableHead>
                              <TableHead>Cadence</TableHead>
                              <TableHead>Next run</TableHead>
                              <TableHead className="cron-ui-cell-numeric">
                                Runs
                              </TableHead>
                              <TableHead className="cron-ui-cell-actions">
                                <span className="cron-ui-sr">Actions</span>
                              </TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {orderedTasks.map((task) => (
                              <TaskRow
                                key={task.subscriptionId}
                                task={task}
                                now={now}
                                selected={
                                  inspector?.kind === 'task' &&
                                  inspector.task.subscriptionId ===
                                    task.subscriptionId
                                }
                                onOpen={() =>
                                  openInspector({ kind: 'task', task })
                                }
                                onReplace={() => requestTaskChange(task)}
                                onRemove={() => void removeTask(task)}
                                onCopyId={() =>
                                  void copyId(task.subscriptionId)
                                }
                                onOpenConversation={() =>
                                  openConversation(task.sessionId)
                                }
                              />
                            ))}
                          </TableBody>
                        </Table>
                      </TableFrame>
                    </TableViewport>
                  )}
                </TabsContent>

                <TabsContent value="bindings" className="cron-ui-tab-content">
                  {bindingsError ? (
                    <StatusPanel
                      variant="alert"
                      headline="Could not read system bindings"
                      detail={bindingsError}
                    />
                  ) : null}
                  {orderedBindings.length === 0 ? (
                    <EmptyState
                      icon={ClockIcon}
                      title="No system bindings"
                      description="Workers that schedule their own functions appear here."
                    />
                  ) : narrow ? (
                    <List>
                      {orderedBindings.map((binding) => (
                        <BindingListRow
                          key={binding.id}
                          binding={binding}
                          now={now}
                          selected={
                            inspector?.kind === 'binding' &&
                            inspector.binding.id === binding.id
                          }
                          onOpen={() =>
                            openInspector({ kind: 'binding', binding })
                          }
                        />
                      ))}
                    </List>
                  ) : (
                    <TableViewport>
                      <TableFrame>
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>Status</TableHead>
                              <TableHead>Owner</TableHead>
                              <TableHead>Function</TableHead>
                              <TableHead>Cadence</TableHead>
                              <TableHead>Next run</TableHead>
                              <TableHead className="cron-ui-cell-numeric">
                                Runs
                              </TableHead>
                              <TableHead className="cron-ui-cell-actions">
                                <span className="cron-ui-sr">Actions</span>
                              </TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {orderedBindings.map((binding) => (
                              <BindingRow
                                key={binding.id}
                                binding={binding}
                                now={now}
                                selected={
                                  inspector?.kind === 'binding' &&
                                  inspector.binding.id === binding.id
                                }
                                onOpen={() =>
                                  openInspector({ kind: 'binding', binding })
                                }
                              />
                            ))}
                          </TableBody>
                        </Table>
                      </TableFrame>
                    </TableViewport>
                  )}
                </TabsContent>
              </Tabs>
            </div>
          )}
        </PageMain>
        {showDetail && !narrow ? (
          <PageSidebar width={380} aria-label="Schedule details">
            <div className="cron-ui-detail-bar">
              <span className="cron-ui-detail-eyebrow">
                {inspector?.kind === 'new'
                  ? 'New schedule'
                  : inspector?.kind === 'task'
                    ? 'Schedule'
                    : 'System binding'}
              </span>
              <IconButton label="Close details" onClick={closeInspector}>
                <CloseIcon className={uiClasses.icon} />
              </IconButton>
            </div>
            {detailScroll}
          </PageSidebar>
        ) : null}
      </PageBody>
    </PageShell>
  )
}
