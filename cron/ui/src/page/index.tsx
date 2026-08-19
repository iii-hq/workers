import {
  Button,
  CodeEditor,
  EmptyState,
  Select,
  type Host,
  IconButton,
  Input,
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
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  errorMessage,
  type FunctionSummary,
  listFunctions,
  listSessionCronTasks,
  listSystemCronBindings,
  removeSessionCronTask,
  resolveModel,
  sendToSession,
  type SessionCronTask,
  type SystemCronBinding,
} from '../lib/api'
import { describeCron, nextCronRun, untilLabel, validateCron } from '../lib/cron'
import { BindingDetail, TaskDetail } from './detail'
import { BindingRow, cadenceLabel, TaskRow } from './rows'
import {
  byNextRun,
  countByStatus,
  type Filter,
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

function ClockIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="8.5" stroke="currentColor" strokeWidth="1.7" />
      <path d="M12 7.5v5l3.3 2" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function RefreshIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M19 8a7.5 7.5 0 1 0 .2 7.7M19 4v4h-4" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function SearchIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="10.8" cy="10.8" r="6.3" stroke="currentColor" strokeWidth="1.7" />
      <path d="m16 16 4 4" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
    </svg>
  )
}

function PlusIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  )
}

function ChevronIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="m9 6 6 6-6 6" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}

function SparkIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" className={className} aria-hidden fill="currentColor">
      <path d="M8 1.5l1.2 3.4 3.3 1.2-3.3 1.2L8 10.7 6.8 7.3 3.5 6.1l3.3-1.2L8 1.5zM12.5 10l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6.6-1.6z" />
    </svg>
  )
}

function CloseIcon({ className }: { className?: string }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="m7 7 10 10M17 7 7 17" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
    </svg>
  )
}

function formatTimestamp(value: number): string {
  if (!value) return 'unknown'
  const milliseconds = value < 10_000_000_000 ? value * 1000 : value
  const date = new Date(milliseconds)
  if (Number.isNaN(date.getTime())) return 'unknown'
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZone: 'UTC',
    timeZoneName: 'short',
  }).format(date)
}

function taskTitle(task: SessionCronTask): string {
  return task.label || (task.target ? task.target : 'Scheduled conversation wake')
}

function taskSchedule(task: SessionCronTask, now: Date): string {
  const description = describeCron(task.expression) ?? task.expression
  const next = nextCronRun(task.expression, now)
  return next ? `${description} · next ${untilLabel(next, now)}` : description
}

export function CronSchedulesPage({
  host,
  panelSide = 'left',
  onRequestClose,
  conversationId,
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
  const [replacementTask, setReplacementTask] = useState<SessionCronTask | null>(null)
  const [statusFilter, setStatusFilter] = useState<Filter>('all')
  const [narrow, setNarrow] = useState(false)
  const [now, setNow] = useState(() => new Date())
  const instanceId = useRef(++mountSequence)
  const tasksRequest = useRef(0)
  const bindingsRequest = useRef(0)
  const sendRequest = useRef(0)
  const currentConversation = useRef(conversationId)
  const layoutRef = useRef<HTMLDivElement | null>(null)
  const inspectorRef = useRef<HTMLElement | null>(null)
  const lastFocus = useRef<HTMLElement | null>(null)
  currentConversation.current = conversationId

  const openInspector = useCallback((next: Exclude<Inspector, null>) => {
    lastFocus.current = document.activeElement instanceof HTMLElement
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
    if (!conversationId) {
      setTasks([])
      setTasksError(null)
      return
    }
    setTasksLoading(true)
    try {
      const next = await listSessionCronTasks(host, conversationId)
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
    if (functionsResult.status === 'fulfilled') setFunctions(functionsResult.value)
    setBindingsLoading(false)
  }, [host])

  const refreshAll = useCallback(async () => {
    await Promise.all([refreshTasks(), refreshBindings()])
    setNow(new Date())
  }, [refreshTasks, refreshBindings])

  useEffect(() => {
    tasksRequest.current += 1
    sendRequest.current += 1
    setTasks([])
    setTasksError(null)
    setTasksLoading(false)
    setComposer('')
    setSending(false)
    setReplacementTask(null)
    setInspector(null)
    setFeedback(null)
  }, [conversationId])

  useEffect(() => {
    void refreshAll()
  }, [refreshAll])

  useEffect(() => {
    const interval = window.setInterval(() => setNow(new Date()), 30_000)
    return () => window.clearInterval(interval)
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

  const inspectorIdentity = inspector?.kind === 'task'
    ? `task:${inspector.task.subscriptionId}`
    : inspector?.kind === 'binding'
      ? `binding:${inspector.binding.id}`
      : inspector?.kind ?? ''

  useEffect(() => {
    if (!narrow || !inspectorIdentity) return
    const frame = window.requestAnimationFrame(() => {
      inspectorRef.current
        ?.querySelector<HTMLElement>('button, input, textarea, [tabindex]:not([tabindex="-1"])')
        ?.focus()
    })
    return () => window.cancelAnimationFrame(frame)
  }, [narrow, inspectorIdentity])

  useEffect(() => {
    setInspector((current) => {
      if (current?.kind === 'task') {
        const next = tasks.find((task) => task.subscriptionId === current.task.subscriptionId)
        if (next) return next === current.task ? current : { kind: 'task', task: next }
        return tasksLoading ? current : null
      }
      if (current?.kind === 'binding') {
        const next = bindings.find((binding) => binding.id === current.binding.id)
        if (next) return next === current.binding ? current : { kind: 'binding', binding: next }
        return bindingsLoading ? current : null
      }
      return current
    })
  }, [tasks, bindings, tasksLoading, bindingsLoading])

  useEffect(() => {
    if (!conversationId) return
    const functionId = `iii::cron-ui::triggers-changed:${instanceId.current}`
    const offHandler = host.iii.on<{ session_id?: string }>(functionId, (event) => {
      if (event?.session_id === conversationId) void refreshTasks()
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'harness::triggers-changed',
      function_id: `${functionId}::${host.iii.browserId}`,
      config: { session_id: conversationId },
    })
    return () => {
      try {
        offTrigger()
      } finally {
        offHandler()
      }
    }
  }, [host, conversationId, refreshTasks])

  const filteredTasks = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return tasks
    return tasks.filter((task) =>
      [taskTitle(task), task.expression, task.target, task.subscriptionId]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle)),
    )
  }, [tasks, query])

  const filteredBindings = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return bindings
    return bindings.filter((binding) =>
      [binding.functionId, binding.expression, binding.workerName, binding.id]
        .some((value) => value.toLowerCase().includes(needle)),
    )
  }, [bindings, query])

  const submitNaturalTask = async () => {
    const taskRequest = composer.trim()
    const sessionId = conversationId
    if (!sessionId || !taskRequest || sending) return
    const request = ++sendRequest.current
    const replacing = replacementTask
    setSending(true)
    setFeedback(null)
    try {
      const model = resolveModel()
      if (!model) {
        setFeedback({
          tone: 'alert',
          message:
            'This conversation has not chosen a model yet. Pick one in the chat composer, then schedule.',
        })
        setSending(false)
        return
      }
      await sendToSession(
        host,
        sessionId,
        replacing
          ? [
              `Replace scheduled task ${replacing.subscriptionId} for this conversation.`,
              'Inspect the existing subscription if the request does not repeat every unchanged detail.',
              'First call engine::register_trigger with the complete replacement, trigger_type "cron", and a valid six- or seven-field Rust cron expression in UTC.',
              `The replacement must return a subscription_id different from the existing id "${replacing.subscriptionId}". If it returns the same id, the registration was deduplicated: do not unregister it and report that the task is unchanged.`,
              `Only after registration succeeds with a different subscription_id, call engine::unregister_trigger with {"id":"${replacing.subscriptionId}"}.`,
              'If replacement registration fails, leave the existing subscription registered. Report both operation results. Do not merely explain the steps.',
              '',
              `Requested replacement: ${taskRequest}`,
            ].join('\n')
          : [
              'Create a scheduled task for this conversation.',
              'Use engine::register_trigger with trigger_type "cron" and a valid six- or seven-field Rust cron expression in UTC.',
              'For a conversation reminder or routine, omit function_id so each fire wakes this conversation, and set a concise label that preserves the task intent.',
              'If the request is sufficiently precise, perform the registration now and report its subscription_id. Do not only describe the steps.',
              '',
              `Request: ${taskRequest}`,
            ].join('\n'),
        model,
      )
      if (request !== sendRequest.current || currentConversation.current !== sessionId) return
      setComposer('')
      setReplacementTask(null)
      setFeedback({
        tone: 'success',
        message: replacing
          ? 'Safe replacement request sent. The old task is retired only after the new registration succeeds.'
          : 'Request sent to Harness. This list updates when the task is registered.',
      })
    } catch (error) {
      if (request !== sendRequest.current || currentConversation.current !== sessionId) return
      setFeedback({ tone: 'alert', message: errorMessage(error) })
    } finally {
      if (request === sendRequest.current && currentConversation.current === sessionId) {
        setSending(false)
      }
    }
  }

  const sendManualTask = async (spec: ManualTaskSpec) => {
    const sessionId = conversationId
    if (!sessionId) return
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
      const model = resolveModel()
      if (!model) {
        setFeedback({
          tone: 'alert',
          message:
            'This conversation has not chosen a model yet. Pick one in the chat composer, then schedule.',
        })
        setSending(false)
        return
      }
      await sendToSession(
        host,
        sessionId,
        [
          'Create this scheduled task for the current conversation.',
          'Call engine::register_trigger exactly once with the JSON object below. Do not substitute a timer and do not merely explain the call.',
          'The expression is UTC. Report the returned subscription_id and any registration note.',
          '',
          JSON.stringify(registration, null, 2),
        ].join('\n'),
        model,
      )
      if (request !== sendRequest.current || currentConversation.current !== sessionId) return
      closeInspector()
      setFeedback({
        tone: 'success',
        message: 'Registration request sent. Harness will add the task after the active turn completes the call.',
      })
    } catch (error) {
      if (request !== sendRequest.current || currentConversation.current !== sessionId) return
      setFeedback({ tone: 'alert', message: errorMessage(error) })
    } finally {
      if (request === sendRequest.current && currentConversation.current === sessionId) {
        setSending(false)
      }
    }
  }

  const requestTaskChange = (task: SessionCronTask) => {
    if (task.target) {
      setFeedback({
        tone: 'warn',
        message: 'Call tasks cannot be replaced safely because Harness does not expose their stored payload template. Remove and recreate this task with the complete call settings.',
      })
      return
    }
    setView('tasks')
    setReplacementTask(task)
    setComposer('')
    setInspector(null)
    setFeedback({
      tone: 'warn',
      message: 'Describe the complete replacement. Harness will retire the old task only after the new registration succeeds.',
    })
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLInputElement>('[data-iii-ui="cron"] input[aria-label="Schedule a task in plain language"]')?.focus()
    })
  }

  const removeTask = async (task: SessionCronTask) => {
    const sessionId = conversationId
    if (!sessionId) return
    try {
      const removed = await removeSessionCronTask(host, sessionId, task.subscriptionId)
      if (currentConversation.current !== sessionId) return
      setFeedback({
        tone: removed ? 'success' : 'warn',
        message: removed ? 'Scheduled task removed.' : 'The task was already retired.',
      })
      closeInspector()
      await refreshTasks()
    } catch (error) {
      setFeedback({ tone: 'alert', message: errorMessage(error) })
    }
  }

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
    const focusable = [...inspectorRef.current.querySelectorAll<HTMLElement>(
      'button, input, textarea, select, [href], [tabindex]:not([tabindex="-1"])',
    )].filter((element) => !element.matches(':disabled') && element.getClientRects().length > 0)
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

  const counts = countByStatus(tasks, now.getTime())
  const orderedTasks = useMemo(() => {
    const next = (task: SessionCronTask) => nextCronRun(task.expression, now)
    return [...tasks]
      .filter((task) => matchesFilter(task, statusFilter, now.getTime()))
      .filter((task) => matchesQuery(task, cadenceLabel(task.expression), query))
      .sort(byNextRun(next))
  }, [tasks, statusFilter, query, now])

  const orderedBindings = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return bindings.filter((binding) =>
      !needle ||
      [binding.functionId, binding.workerName, binding.expression, binding.id]
        .some((value) => value.toLowerCase().includes(needle)),
    )
  }, [bindings, query])

  const loading = view === 'tasks' ? tasksLoading : bindingsLoading
  const error = view === 'tasks' ? tasksError : bindingsError
  const showDetail = inspector !== null
  const detailOnly = narrow && showDetail

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
      />
    ) : (
      <BindingDetail binding={inspector.binding} now={now} />
    )

  return (
    <PageShell className="cron-ui-shell">
      <PageHeader
        icon={<ClockIcon />}
        title="Cron"
        description="Schedule functions and monitor every run."
        actions={
          <div className="cron-ui-header-actions">
            <label className="cron-ui-search">
              <SearchIcon className="cron-ui-icon" />
              <Input
                value={query}
                onChange={setQuery}
                placeholder="Search schedules"
                aria-label="Search schedules"
              />
            </label>
            <IconButton
              label="Refresh schedules"
              onClick={() => void refreshAll()}
              disabled={tasksLoading || bindingsLoading}
            >
              <RefreshIcon
                className={loading ? 'cron-ui-icon cron-ui-spin' : 'cron-ui-icon'}
              />
            </IconButton>
            <Button
              variant="primary"
              size="sm"
              disabled={!conversationId}
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
            <section className="cron-ui-detail-page" aria-label="Schedule details">
              <div className="cron-ui-detail-bar">
                <Button variant="ghost" size="sm" onClick={closeInspector}>
                  <ChevronIcon className="cron-ui-icon cron-ui-back" />
                  All schedules
                </Button>
              </div>
              <div
                className="cron-ui-detail-scroll"
                ref={inspectorRef as React.RefObject<HTMLDivElement>}
                onKeyDown={handleInspectorKeyDown}
              >
                {detailBody}
              </div>
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
                <SparkIcon className="cron-ui-icon cron-ui-composer-icon" />
                <Input
                  value={composer}
                  onChange={setComposer}
                  placeholder={
                    replacementTask
                      ? `Describe the replacement for ${replacementTask.label ?? 'this schedule'}…`
                      : 'Describe a routine to schedule…'
                  }
                  aria-label="Describe a routine to schedule"
                  disabled={!conversationId || sending}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="sm"
                  disabled={!conversationId || sending || composer.trim().length === 0}
                >
                  {sending ? 'Scheduling…' : 'Schedule'}
                </Button>
              </form>

              {feedback ? (
                <StatusPanel
                  variant={
                    feedback.tone === 'success'
                      ? 'success'
                      : feedback.tone === 'warn'
                        ? 'warn'
                        : 'alert'
                  }
                  headline={feedback.message}
                />
              ) : null}

              {!conversationId ? (
                <StatusPanel
                  variant="info"
                  headline="Open this page beside a conversation"
                  detail="Scheduled tasks belong to a conversation, so the page needs one to read or create them. System bindings below stay visible."
                />
              ) : null}

              <Tabs value={view} onValueChange={(value) => setView(value as View)}>
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
                        { value: 'all', label: `All ${counts.all}`, icon: false },
                        { value: 'active', label: `Active ${counts.active}`, icon: false },
                        { value: 'ending', label: `Ending soon ${counts.ending}`, icon: false },
                        { value: 'finished', label: `Finished ${counts.finished}`, icon: false },
                      ]}
                    />
                  ) : null}
                </div>

                <TabsContent value="tasks" className="cron-ui-tab-content">
                  {error ? (
                    <StatusPanel variant="alert" headline="Could not read schedules" detail={error} />
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
                        tasks.length === 0 ? 'No schedules yet' : 'Nothing matches'
                      }
                      description={
                        tasks.length === 0
                          ? 'Describe a routine above, or write the cron expression yourself.'
                          : 'Change the filter or clear the search to see the rest.'
                      }
                      action={
                        tasks.length === 0 && conversationId
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
                              <TableHead className="cron-ui-cell-numeric">Fires</TableHead>
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
                                  inspector.task.subscriptionId === task.subscriptionId
                                }
                                onOpen={() => openInspector({ kind: 'task', task })}
                                onReplace={() => requestTaskChange(task)}
                                onRemove={() => void removeTask(task)}
                                onCopyId={() => void copyId(task.subscriptionId)}
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
                              <TableHead className="cron-ui-cell-numeric">Fires</TableHead>
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
                                onOpen={() => openInspector({ kind: 'binding', binding })}
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
                <CloseIcon className="cron-ui-icon" />
              </IconButton>
            </div>
            <div
              className="cron-ui-detail-scroll"
              ref={inspectorRef as React.RefObject<HTMLDivElement>}
              onKeyDown={handleInspectorKeyDown}
            >
              {detailBody}
            </div>
          </PageSidebar>
        ) : null}
      </PageBody>
    </PageShell>
  )
}

interface ManualTaskSpec {
  label: string
  expression: string
  delivery: 'notify' | 'call'
  functionId?: string
  payload?: unknown
  eventInto?: string
  maxFires?: number
}

const PRESETS = [
  { value: 'hourly', label: 'Every hour', expression: '0 0 * * * *' },
  { value: 'daily', label: 'Daily at 09:00 UTC', expression: '0 0 9 * * *' },
  { value: 'weekdays', label: 'Weekdays at 09:00 UTC', expression: '0 0 9 * * 2-6' },
  { value: 'weekly', label: 'Mondays at 09:00 UTC', expression: '0 0 9 * * 2' },
  { value: 'custom', label: 'Custom expression', expression: '' },
] as const

function ManualTaskForm({
  functions,
  sending,
  onClose,
  onSubmit,
}: {
  functions: FunctionSummary[]
  sending: boolean
  onClose: () => void
  onSubmit: (spec: ManualTaskSpec) => void
}) {
  const [label, setLabel] = useState('')
  const [preset, setPreset] = useState('daily')
  const [expression, setExpression] = useState('0 0 9 * * *')
  const [delivery, setDelivery] = useState<'notify' | 'call'>('notify')
  const [functionId, setFunctionId] = useState<string | undefined>()
  const [payloadText, setPayloadText] = useState('{}')
  const [eventInto, setEventInto] = useState('/cron_event')
  const [maxFires, setMaxFires] = useState('')
  const [formError, setFormError] = useState<string | null>(null)

  const submit = () => {
    if (!label.trim()) {
      setFormError('Describe what the task should do.')
      return
    }
    const cronError = validateCron(expression)
    if (cronError) {
      setFormError(cronError)
      return
    }
    if (delivery === 'call' && !functionId) {
      setFormError('Choose the function that should receive each cron event.')
      return
    }
    let payload: unknown = undefined
    if (delivery === 'call') {
      try {
        payload = JSON.parse(payloadText)
      } catch {
        setFormError('Fixed payload must be valid JSON.')
        return
      }
    }
    const max = maxFires.trim() ? Number(maxFires) : undefined
    if (max !== undefined && (!Number.isInteger(max) || max < 1)) {
      setFormError('Maximum fires must be a positive whole number.')
      return
    }
    setFormError(null)
    onSubmit({
      label: label.trim(),
      expression: expression.trim(),
      delivery,
      functionId,
      payload,
      eventInto: eventInto.trim(),
      maxFires: max,
    })
  }

  return (
    <div className="cron-ui-inspector-inner cron-ui-form-panel">
      <header className="cron-ui-detail-head">
        <h2 className="cron-ui-detail-title">New schedule</h2>
      </header>
      <p className="cron-ui-inspector-copy">
        The active agent registers the task so Harness can persist ownership, lifecycle, and fire count.
      </p>

      <div className="cron-ui-field">
        <label htmlFor="cron-task-label">task</label>
        <textarea
          id="cron-task-label"
          value={label}
          onChange={(event) => setLabel(event.target.value)}
          placeholder="Prepare a daily brief of open work"
          rows={3}
        />
      </div>

      <div className="cron-ui-field">
        <label>schedule</label>
        <Select
          value={preset}
          options={PRESETS.map((item) => ({ value: item.value, label: item.label }))}
          onChange={(value) => {
            setPreset(value)
            const selected = PRESETS.find((item) => item.value === value)
            if (selected?.expression) setExpression(selected.expression)
          }}
          aria-label="Schedule preset"
        />
      </div>

      <div className="cron-ui-field">
        <label htmlFor="cron-task-expression">cron expression</label>
        <Input
          id="cron-task-expression"
          value={expression}
          onChange={(value) => {
            setExpression(value)
            setPreset('custom')
          }}
          preserveCase
          placeholder="0 0 9 * * *"
        />
        <span className="cron-ui-hint">sec min hour day month weekday [year] · UTC</span>
      </div>

      <div className="cron-ui-field">
        <label>delivery</label>
        <Select
          value={delivery}
          options={[
            { value: 'notify', label: 'Wake this conversation' },
            { value: 'call', label: 'Call a function' },
          ]}
          onChange={(value) => setDelivery(value)}
          aria-label="Task delivery"
        />
        <span className="cron-ui-hint">
          A conversation wake lets the active agent perform the routine. Function calls run mechanically without starting an agent.
        </span>
      </div>

      {delivery === 'call' ? (
        <>
          <div className="cron-ui-field">
            <label>target function</label>
            <Select
              value={functionId}
              options={functions.map((fn) => ({
                value: fn.functionId,
                label: fn.functionId,
                title: fn.description,
              }))}
              onChange={setFunctionId}
              placeholder="Choose a function"
              aria-label="Target function"
            />
          </div>
          <div className="cron-ui-field">
            <label htmlFor="cron-task-event-path">event JSON pointer</label>
            <Input
              id="cron-task-event-path"
              value={eventInto}
              onChange={setEventInto}
              preserveCase
              placeholder="/cron_event"
            />
          </div>
          <div className="cron-ui-field cron-ui-editor-field">
            <label>fixed payload</label>
            <CodeEditor
              value={payloadText}
              onChange={setPayloadText}
              language="json"
              className="cron-ui-code-editor"
              aria-label="Fixed target payload"
            />
          </div>
        </>
      ) : null}

      <div className="cron-ui-field">
        <label htmlFor="cron-task-max-fires">maximum fires</label>
        <input
          id="cron-task-max-fires"
          type="number"
          min={1}
          step={1}
          value={maxFires}
          onChange={(event) => setMaxFires(event.target.value)}
          placeholder="unbounded"
        />
        <span className="cron-ui-hint">Leave empty for a recurring task that runs until removed.</span>
      </div>

      {formError ? <div className="cron-ui-form-error" role="alert">{formError}</div> : null}

      <div className="cron-ui-form-actions">
        <Button variant="ghost" size="sm" onClick={onClose}>cancel</Button>
        <Button variant="primary" size="sm" disabled={sending} onClick={submit}>
          {sending ? 'sending' : 'create task'}
        </Button>
      </div>
    </div>
  )
}
