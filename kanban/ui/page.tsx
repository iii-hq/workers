import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  EmptyState,
  type Host,
  Input,
  List,
  ListGroup,
  ListGroupLabel,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Select,
  Skeleton,
  StatusDot,
  StatusPanel,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  uiClasses,
} from '@iii-dev/console-ui'
import { type FormEvent, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react'

type TaskStatus = 'needs_you' | 'queued' | 'running' | 'review' | 'ready' | 'done'
type RunStatus = 'needs_you' | 'active' | 'review' | 'ready' | 'done'
type IsolationMode = 'shared' | 'worktree_per_task'

type Executor = {
  id: string
  label: string
  kind: 'task' | 'harness'
  function_id: string
  worker_name: string
  stop_function?: string
  available: boolean
}

type GitStatus = {
  clean: boolean
  ahead: number
  behind: number
  staged: number
  unstaged: number
  untracked: number
  conflicted: number
  diffstat: string
  unpushed: number
  in_rebase: boolean
  head_sha: string
  integrated?: boolean
}

type Run = {
  id: string
  title: string
  goal?: string
  cwd?: string
  repo_path?: string
  base_ref?: string
  target_branch?: string
  isolation?: IsolationMode
  root_session_id?: string
  task_ids: string[]
  source: 'kanban' | 'harness'
  status: RunStatus
  counts: Record<TaskStatus, number>
  created_at_ms: number
  updated_at_ms: number
}

type Task = {
  id: string
  key?: string
  run_id: string
  run_title: string
  title: string
  instruction: string
  executor_id: string
  executor_function: string
  executor_kind: 'task' | 'harness'
  cwd?: string
  depends_on?: string[]
  model?: string
  profile?: string
  worktree_id?: string
  worktree_path?: string
  branch?: string
  base_ref?: string
  base_sha?: string
  dev_port?: number
  land_job_id?: string
  land_status?: 'landing' | 'landed' | 'blocked'
  git_status?: GitStatus
  status: TaskStatus
  attempt: number
  external_session_id?: string
  external_turn_id?: string
  source: 'kanban' | 'harness'
  result?: unknown
  error?: string
  created_at_ms: number
  updated_at_ms: number
}

type Capabilities = { harness: boolean; worktree: boolean; external_executors: number }
type ModelOption = { id: string; label: string; provider: string }
type Board = {
  runs: Run[]
  tasks: Task[]
  executors: Executor[]
  models: ModelOption[]
  capabilities: Capabilities
  updated_at_ms: number
}
type ChangedEvent = { kind?: 'run' | 'task'; id?: string; run_id?: string; updated_at_ms?: number }
type Props = PageRenderProps & { host: Host }
type Feedback = { kind: 'success' | 'error'; text: string }
type DraftTask = {
  id: string
  key: string
  title: string
  instruction: string
  executor: string
  dependsOn: string
  model: string
  profile: string
}

const EVENTS_FN = 'iii::kanban::changed'
const STATUS_LABEL: Record<TaskStatus, string> = {
  needs_you: 'Needs you',
  queued: 'Queued',
  running: 'Running',
  review: 'Review',
  ready: 'Ready',
  done: 'Done',
}

function describe(cause: unknown): string {
  if (cause instanceof Error) return cause.message.replace(/^handler error:\s*/i, '')
  if (cause && typeof cause === 'object') {
    const message = (cause as { message?: unknown }).message
    if (typeof message === 'string') return message.replace(/^handler error:\s*/i, '')
    try {
      return JSON.stringify(cause)
    } catch {
      return String(cause)
    }
  }
  return String(cause)
}

function resultText(result: unknown): string {
  if (result === undefined || result === null) return ''
  if (typeof result === 'string') return result
  try {
    return JSON.stringify(result, null, 2)
  } catch {
    return String(result)
  }
}

function relativeTime(timestamp: number): string {
  const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000))
  if (seconds < 10) return 'now'
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h`
  return `${Math.floor(hours / 24)}d`
}

function statusBadge(status: TaskStatus | RunStatus): 'default' | 'ok' | 'warn' | 'alert' | 'accent' {
  if (status === 'needs_you') return 'alert'
  if (status === 'running' || status === 'active') return 'accent'
  if (status === 'review') return 'warn'
  if (status === 'done') return 'ok'
  return 'default'
}

function statusTone(status: TaskStatus | RunStatus): 'accent' | 'alert' | 'warn' | 'ink' {
  if (status === 'needs_you') return 'alert'
  if (status === 'running' || status === 'active') return 'accent'
  if (status === 'review') return 'warn'
  return 'ink'
}

function shortPath(path?: string): string {
  if (!path) return 'Shared directory'
  const parts = path.split('/').filter(Boolean)
  return parts.length > 2 ? `…/${parts.slice(-2).join('/')}` : path
}

function Field({ id, label, hint, children }: { id: string; label: string; hint?: string; children: ReactNode }) {
  return (
    <div className={uiClasses.field}>
      <label id={`${id}-label`} htmlFor={id} className={uiClasses.fieldLabel}>
        {label}
      </label>
      {children}
      {hint ? <span className={uiClasses.fieldDescription}>{hint}</span> : null}
    </div>
  )
}

function newDraft(index: number, executors: Executor[], defaultModel?: string | null): DraftTask {
  const executor = executors[index % Math.max(1, executors.length)]
  const id = globalThis.crypto.randomUUID()
  return {
    id,
    key: `task-${id.slice(0, 8)}`,
    title: '',
    instruction: '',
    executor: executor?.id ?? '',
    dependsOn: '',
    model: executor?.kind === 'harness' ? (defaultModel ?? '') : '',
    profile: '',
  }
}

function NewRunDialog({
  open,
  executors,
  models,
  defaultModel,
  capabilities,
  initialRepo,
  recentDirectories,
  busy,
  error,
  onOpenChange,
  onCreate,
}: {
  open: boolean
  executors: Executor[]
  models: ModelOption[]
  defaultModel?: string | null
  capabilities: Capabilities
  initialRepo?: string | null
  recentDirectories: string[]
  busy: boolean
  error?: string | null
  onOpenChange: (open: boolean) => void
  onCreate: (input: Record<string, unknown>) => Promise<void>
}) {
  const [title, setTitle] = useState('')
  const [goal, setGoal] = useState('')
  const [repoPath, setRepoPath] = useState(initialRepo ?? recentDirectories[0] ?? '')
  const [baseRef, setBaseRef] = useState('HEAD')
  const [targetBranch, setTargetBranch] = useState('')
  const [isolation, setIsolation] = useState<IsolationMode>(
    capabilities.worktree && Boolean(initialRepo) ? 'worktree_per_task' : 'shared',
  )
  const [autoDispatch, setAutoDispatch] = useState(true)
  const [tasks, setTasks] = useState<DraftTask[]>(() => [newDraft(0, executors, defaultModel)])
  const autoIsolation = useRef(false)

  useEffect(() => {
    const first = executors[0]?.id
    if (!first) return
    const firstExecutor = executors[0]
    setTasks((current) =>
      current.map((task) => {
        if (task.executor) {
          const executor = executors.find((candidate) => candidate.id === task.executor)
          return executor?.kind === 'harness' && !task.model && defaultModel ? { ...task, model: defaultModel } : task
        }
        return {
          ...task,
          executor: first,
          ...(firstExecutor.kind === 'harness' && defaultModel ? { model: defaultModel } : {}),
        }
      }),
    )
  }, [defaultModel, executors])

  useEffect(() => {
    if (!capabilities.worktree && isolation === 'worktree_per_task') setIsolation('shared')
  }, [capabilities.worktree, isolation])

  useEffect(() => {
    if (!open || repoPath.trim()) return
    const nextRepo = initialRepo?.trim() || recentDirectories[0]?.trim()
    if (nextRepo) setRepoPath(nextRepo)
  }, [initialRepo, open, recentDirectories, repoPath])

  useEffect(() => {
    if (!open) {
      autoIsolation.current = false
      return
    }
    if (!autoIsolation.current && capabilities.worktree && repoPath.trim()) {
      autoIsolation.current = true
      setIsolation('worktree_per_task')
    }
  }, [capabilities.worktree, open, repoPath])

  const updateTask = (id: string, patch: Partial<DraftTask>) => {
    setTasks((current) => current.map((task) => (task.id === id ? { ...task, ...patch } : task)))
  }

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    await onCreate({
      title,
      goal,
      cwd: repoPath,
      repo_path: repoPath,
      base_ref: baseRef,
      target_branch: targetBranch,
      isolation,
      auto_dispatch: autoDispatch,
      tasks: tasks.map((task) => ({
        key: task.key,
        title: task.title,
        instruction: task.instruction,
        executor: task.executor,
        depends_on: task.dependsOn ? [task.dependsOn] : [],
        model: task.model,
        profile: task.profile,
      })),
    })
  }

  const invalid =
    executors.length === 0 ||
    tasks.some(
      (task) =>
        !task.instruction.trim() ||
        !task.executor ||
        (executors.find((executor) => executor.id === task.executor)?.kind === 'harness' && !task.model),
    ) ||
    (isolation === 'worktree_per_task' && !repoPath.trim())

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="kb-run-dialog">
        <DialogTitle>New Kanban run</DialogTitle>
        <DialogDescription>
          One root session, child executors, dependency gates, and optional isolated Git worktrees.
        </DialogDescription>
        <form className="kb-run-form" onSubmit={submit}>
          <div className="kb-run-grid">
            <Field id="kb-run-title" label="Run name">
              <Input id="kb-run-title" value={title} onChange={setTitle} placeholder="Uses the first task when empty" />
            </Field>
            <Field id="kb-run-goal" label="Goal">
              <Input id="kb-run-goal" value={goal} onChange={setGoal} placeholder="What this run should produce" />
            </Field>
            <Field id="kb-run-repo" label="Repository" hint="Parent checkout used to create worktrees.">
              <Input
                id="kb-run-repo"
                value={repoPath}
                onChange={setRepoPath}
                placeholder="/absolute/path/to/repository"
                list="kb-recent-repositories"
              />
              <datalist id="kb-recent-repositories">
                {recentDirectories.map((path) => (
                  <option key={path} value={path} />
                ))}
              </datalist>
            </Field>
            <Field id="kb-run-isolation" label="Workspace">
              <Select
                value={isolation}
                options={[
                  { value: 'worktree_per_task', label: 'One worktree per task', disabled: !capabilities.worktree },
                  { value: 'shared', label: 'Shared directory' },
                ]}
                onChange={setIsolation}
                aria-label="Workspace isolation"
              />
            </Field>
            <Field id="kb-run-base" label="Base ref">
              <Input id="kb-run-base" value={baseRef} onChange={setBaseRef} placeholder="HEAD" />
            </Field>
            <Field id="kb-run-target" label="Land onto" hint="Required only when landing reviewed work.">
              <Input id="kb-run-target" value={targetBranch} onChange={setTargetBranch} placeholder="main" />
            </Field>
          </div>

          <div className="kb-draft-header">
            <div>
              <strong>Tasks</strong>
              <span>Each task becomes a child session under this run.</span>
            </div>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={tasks.length >= 24}
              title={tasks.length >= 24 ? 'A run supports at most 24 tasks' : undefined}
              onClick={() => setTasks((current) => [...current, newDraft(current.length, executors, defaultModel)])}
            >
              Add task
            </Button>
          </div>
          <div className="kb-draft-list">
            {tasks.map((task, index) => {
              const previous = tasks.slice(0, index)
              return (
                <section key={task.id} className="kb-draft-task">
                  <div className="kb-draft-index">{String(index + 1).padStart(2, '0')}</div>
                  <div className="kb-draft-fields">
                    <div className="kb-draft-row">
                      <Input
                        value={task.title}
                        onChange={(value) => updateTask(task.id, { title: value })}
                        placeholder="Task title"
                        aria-label={`Task ${index + 1} title`}
                      />
                      <Select
                        value={task.executor || undefined}
                        options={executors.map((executor) => ({ value: executor.id, label: executor.label }))}
                        onChange={(executorId) => {
                          const executor = executors.find((candidate) => candidate.id === executorId)
                          updateTask(task.id, {
                            executor: executorId,
                            ...(executor?.kind === 'harness' && !task.model
                              ? { model: defaultModel ?? models[0]?.id ?? '' }
                              : {}),
                          })
                        }}
                        placeholder="Executor"
                        aria-label={`Task ${index + 1} executor`}
                      />
                      <Select
                        value={task.dependsOn || undefined}
                        options={previous.map((candidate) => ({
                          value: candidate.key,
                          label: `After ${candidate.title || candidate.key}`,
                        }))}
                        onChange={(dependsOn) => updateTask(task.id, { dependsOn })}
                        onClear={() => updateTask(task.id, { dependsOn: '' })}
                        allowEmpty
                        emptyLabel="Starts immediately"
                        placeholder="Starts immediately"
                        disabled={previous.length === 0}
                        aria-label={`Task ${index + 1} dependency`}
                      />
                      {tasks.length > 1 ? (
                        <Button
                          type="button"
                          size="sm"
                          variant="ghost"
                          onClick={() =>
                            setTasks((current) =>
                              current
                                .filter((item) => item.id !== task.id)
                                .map((item) => (item.dependsOn === task.key ? { ...item, dependsOn: '' } : item)),
                            )
                          }
                        >
                          Remove
                        </Button>
                      ) : null}
                    </div>
                    <textarea
                      value={task.instruction}
                      onChange={(event) => updateTask(task.id, { instruction: event.currentTarget.value })}
                      placeholder="Self-contained task instruction"
                      rows={3}
                      aria-label={`Task ${index + 1} instruction`}
                    />
                    <div className="kb-draft-row kb-draft-row-secondary">
                      <Input
                        value={task.profile}
                        onChange={(value) => updateTask(task.id, { profile: value })}
                        placeholder="Agent or persona (optional)"
                        aria-label={`Task ${index + 1} agent or persona`}
                      />
                      <Select
                        value={task.model || undefined}
                        options={models.map((model) => ({
                          value: model.id,
                          label: model.label,
                          title: model.provider,
                        }))}
                        onChange={(model) => updateTask(task.id, { model })}
                        onClear={() => updateTask(task.id, { model: '' })}
                        allowEmpty
                        emptyLabel="No model override"
                        placeholder={
                          executors.find((candidate) => candidate.id === task.executor)?.kind === 'harness'
                            ? 'Model required'
                            : 'Model override'
                        }
                        aria-label={`Task ${index + 1} model`}
                      />
                    </div>
                  </div>
                </section>
              )
            })}
          </div>
          {error ? (
            <div className="kb-dialog-error" role="alert">
              {error}
            </div>
          ) : null}
          <div className="kb-dialog-foot">
            <label className="kb-checkbox">
              <input
                type="checkbox"
                checked={autoDispatch}
                onChange={(event) => setAutoDispatch(event.currentTarget.checked)}
              />
              Dispatch eligible tasks immediately
            </label>
            <div>
              <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" disabled={busy || invalid}>
                {busy ? 'Creating…' : `Create ${tasks.length} task${tasks.length === 1 ? '' : 's'}`}
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}

function RunSidebar({
  runs,
  activeRunId,
  onSelect,
}: {
  runs: Run[]
  activeRunId: string | null
  onSelect: (id: string) => void
}) {
  const active = runs.filter((run) => run.status !== 'done')
  const history = runs.filter((run) => run.status === 'done')
  const group = (label: string, items: Run[]) =>
    items.length ? (
      <ListGroup>
        <ListGroupLabel>{label}</ListGroupLabel>
        {items.map((run) => (
          <ListItem
            key={run.id}
            selected={run.id === activeRunId}
            onClick={() => onSelect(run.id)}
            leading={<StatusDot tone={statusTone(run.status)} pulse={run.status === 'active'} />}
            label={run.title}
            description={
              run.repo_path
                ? shortPath(run.repo_path)
                : run.source === 'harness'
                  ? 'Harness session tree'
                  : 'Shared workspace'
            }
            trailing={<span className="kb-run-count">{run.task_ids.length}</span>}
          />
        ))}
      </ListGroup>
    ) : null
  return (
    <List>
      {group('Active', active)}
      {group('History', history)}
    </List>
  )
}

function GitSummary({ task }: { task: Task }) {
  const git = task.git_status
  if (!task.worktree_id) return <span className="kb-muted">Shared</span>
  if (task.land_status === 'landed') return <Badge variant="ok">Landed</Badge>
  if (task.land_status === 'landing') return <Badge variant="accent">Landing</Badge>
  if (task.land_status === 'blocked') return <Badge variant="alert">Blocked</Badge>
  if (!git) return <span className="kb-muted">Checking</span>
  if (git.conflicted || git.in_rebase) return <Badge variant="alert">Conflict</Badge>
  if (git.clean && git.ahead === 0) return <span className="kb-muted">Clean</span>
  return (
    <span className="kb-git-counts">
      {git.ahead ? `+${git.ahead} commit${git.ahead === 1 ? '' : 's'}` : 'Changes'}
      {git.unstaged || git.untracked ? ` · ${git.unstaged + git.untracked} files` : ''}
    </span>
  )
}

function TaskActions({
  task,
  run,
  executor,
  retryModel,
  busy,
  onAct,
}: {
  task: Task
  run: Run
  executor?: Executor
  retryModel?: string
  busy: boolean
  onAct: (action: string, input?: Record<string, unknown>) => void
}) {
  if (task.source === 'harness') return null
  return (
    <div className="kb-actions">
      {task.status === 'ready' ? (
        <Button size="sm" variant="primary" disabled={busy} onClick={() => onAct('dispatch')}>
          Run
        </Button>
      ) : null}
      {task.status === 'needs_you' ? (
        <Button
          size="sm"
          variant="primary"
          disabled={busy || (task.executor_kind === 'harness' && !retryModel)}
          onClick={() => onAct('retry', retryModel ? { model: retryModel } : {})}
        >
          Retry
        </Button>
      ) : null}
      {task.status === 'running' && executor?.stop_function ? (
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => onAct('stop')}>
          Stop
        </Button>
      ) : null}
      {task.status === 'review' ? (
        <Button size="sm" variant="primary" disabled={busy} onClick={() => onAct('accept')}>
          Accept
        </Button>
      ) : null}
      {task.worktree_id &&
      ['review', 'done'].includes(task.status) &&
      !['landing', 'landed'].includes(task.land_status ?? '') ? (
        <Button
          size="sm"
          variant="ghost"
          disabled={busy || !run.target_branch}
          title={run.target_branch ? `Land onto ${run.target_branch}` : 'Set a target branch on this run'}
          onClick={() => onAct('land')}
        >
          Land
        </Button>
      ) : null}
    </div>
  )
}

const KANBAN_LANES: Array<{ id: string; label: string; description: string; statuses: TaskStatus[] }> = [
  { id: 'ready', label: 'Ready', description: 'Queued or held for launch', statuses: ['queued', 'ready'] },
  { id: 'running', label: 'In progress', description: 'Executing in a child session', statuses: ['running'] },
  { id: 'review', label: 'Review', description: 'Inspect output and Git changes', statuses: ['review'] },
  { id: 'done', label: 'Done', description: 'Accepted or landed', statuses: ['done'] },
]

function KanbanCard({
  task,
  allTasks,
  selected,
  executor,
  onSelect,
}: {
  task: Task
  allTasks: Task[]
  selected: boolean
  executor?: Executor
  onSelect: () => void
}) {
  const dependencies =
    task.depends_on
      ?.map((id) => allTasks.find((candidate) => candidate.id === id))
      .filter((candidate): candidate is Task => Boolean(candidate)) ?? []
  const waiting = dependencies.filter((dependency) => !['review', 'done'].includes(dependency.status))
  const workspace = task.worktree_id ? task.branch || task.worktree_id : 'Shared directory'
  return (
    <article className="kb-card" data-selected={selected ? '' : undefined} data-status={task.status}>
      <button type="button" className="kb-card-open" onClick={onSelect} aria-label={`Open ${task.title}`}>
        <span className="kb-card-title">
          <StatusDot tone={statusTone(task.status)} pulse={task.status === 'running'} />
          <strong>{task.title}</strong>
          <time>{relativeTime(task.updated_at_ms)}</time>
        </span>
        <span className="kb-card-copy">{task.error || task.instruction}</span>
        <span className="kb-card-route">
          <small>ROOT</small>
          <b>→</b>
          <small>{executor?.label ?? task.executor_id}</small>
          {task.profile ? <em>{task.profile}</em> : null}
        </span>
        {dependencies.length ? (
          <span className="kb-card-dependency" data-waiting={waiting.length ? '' : undefined}>
            {waiting.length
              ? `Waiting for ${waiting.map((item) => item.title).join(', ')}`
              : `After ${dependencies.map((item) => item.title).join(', ')}`}
          </span>
        ) : null}
        <span className="kb-card-workspace">
          <span>
            <small>{task.worktree_id ? 'WORKTREE' : 'WORKSPACE'}</small>
            <strong>{workspace}</strong>
          </span>
          <GitSummary task={task} />
        </span>
      </button>
    </article>
  )
}

function KanbanBoard({
  tasks,
  allTasks,
  selectedId,
  executors,
  onSelect,
}: {
  tasks: Task[]
  allTasks: Task[]
  selectedId: string | null
  executors: Map<string, Executor>
  onSelect: (id: string) => void
}) {
  const needsYou = tasks.filter((task) => task.status === 'needs_you')
  return (
    <section className="kb-board" aria-label="Run Kanban board">
      {needsYou.length ? (
        <section className="kb-attention" aria-label="Tasks needing attention">
          <header>
            <span>
              <StatusDot tone="alert" />
              <strong>Needs you</strong>
            </span>
            <b>{needsYou.length}</b>
          </header>
          <div>
            {needsYou.map((task) => (
              <KanbanCard
                key={task.id}
                task={task}
                allTasks={allTasks}
                selected={task.id === selectedId}
                executor={executors.get(task.executor_id)}
                onSelect={() => onSelect(task.id)}
              />
            ))}
          </div>
        </section>
      ) : null}
      <div className="kb-lanes">
        {KANBAN_LANES.map((lane) => {
          const laneTasks = tasks.filter((task) => lane.statuses.includes(task.status))
          return (
            <section key={lane.id} className="kb-lane" data-lane={lane.id}>
              <header>
                <div>
                  <strong>{lane.label}</strong>
                  <span>{lane.description}</span>
                </div>
                <b>{laneTasks.length}</b>
              </header>
              <div className="kb-lane-cards">
                {laneTasks.map((task) => (
                  <KanbanCard
                    key={task.id}
                    task={task}
                    allTasks={allTasks}
                    selected={task.id === selectedId}
                    executor={executors.get(task.executor_id)}
                    onSelect={() => onSelect(task.id)}
                  />
                ))}
                {laneTasks.length === 0 ? <div className="kb-lane-empty">No tasks</div> : null}
              </div>
            </section>
          )
        })}
      </div>
    </section>
  )
}

function Inspector({
  host,
  run,
  task,
  allTasks,
  executor,
  models,
  defaultModel,
  busy,
  onBack,
  onAct,
}: {
  host: Host
  run: Run
  task: Task
  allTasks: Task[]
  executor?: Executor
  models: ModelOption[]
  defaultModel?: string | null
  busy: boolean
  onBack: () => void
  onAct: (action: string, input?: Record<string, unknown>) => void
}) {
  const [retryModel, setRetryModel] = useState(task.model ?? defaultModel ?? models[0]?.id ?? '')
  const output = task.error || resultText(task.result)
  const dependencies =
    task.depends_on?.map((id) => allTasks.find((candidate) => candidate.id === id)?.title).filter(Boolean) ?? []
  const openSession = (id?: string) => id && host.chat?.selectConversation?.(id)
  const openShell = () => {
    const cwd = task.worktree_path ?? task.cwd
    if (cwd) host.panels?.open({ pageId: 'shell', context: { type: 'agent-terminal', cwd, command: '' } })
  }
  useEffect(() => {
    setRetryModel(task.model ?? defaultModel ?? models[0]?.id ?? '')
  }, [defaultModel, models, task.id, task.model])
  return (
    <aside className="kb-inspector" aria-label="Task details">
      <div className="kb-inspector-head">
        <Button size="sm" variant="ghost" onClick={onBack}>
          Back
        </Button>
        <Badge variant={statusBadge(task.status)}>{STATUS_LABEL[task.status]}</Badge>
      </div>
      <div className="kb-inspector-title">
        <h2>{task.title}</h2>
        <span>Attempt {task.attempt}</span>
      </div>
      <div className="kb-route">
        <button type="button" onClick={() => openSession(run.root_session_id)}>
          <small>Root</small>
          <strong>{run.title}</strong>
        </button>
        <span>→</span>
        <button type="button" onClick={() => openSession(task.external_session_id)}>
          <small>Child</small>
          <strong>{executor?.label ?? task.executor_id}</strong>
        </button>
      </div>
      <dl className="kb-facts">
        <div>
          <dt>Repository</dt>
          <dd className="kb-mono">{run.repo_path || task.cwd || 'Not set'}</dd>
        </div>
        <div>
          <dt>Workspace</dt>
          <dd>{task.worktree_id ? 'Managed worktree' : 'Shared directory'}</dd>
        </div>
        {task.worktree_id ? (
          <div>
            <dt>Worktree</dt>
            <dd className="kb-mono">{task.worktree_id}</dd>
          </div>
        ) : null}
        {task.branch ? (
          <div>
            <dt>Branch</dt>
            <dd className="kb-mono">{task.branch}</dd>
          </div>
        ) : null}
        {run.target_branch ? (
          <div>
            <dt>Land target</dt>
            <dd className="kb-mono">{run.target_branch}</dd>
          </div>
        ) : null}
        {dependencies.length ? (
          <div>
            <dt>Starts after</dt>
            <dd>{dependencies.join(', ')}</dd>
          </div>
        ) : null}
        {task.dev_port ? (
          <div>
            <dt>Dev port</dt>
            <dd className="kb-mono">{task.dev_port}</dd>
          </div>
        ) : null}
      </dl>
      {task.status === 'needs_you' && task.executor_kind === 'harness' ? (
        <Field
          id={`kb-retry-model-${task.id}`}
          label="Harness model"
          hint="Top-level child runs must name a model explicitly."
        >
          <Select
            value={retryModel || undefined}
            options={models.map((model) => ({ value: model.id, label: model.label, title: model.provider }))}
            onChange={setRetryModel}
            placeholder="Select model"
            aria-label="Harness retry model"
          />
        </Field>
      ) : null}
      {task.git_status ? (
        <section className="kb-git-panel">
          <div>
            <strong>Git status</strong>
            <GitSummary task={task} />
          </div>
          <div className="kb-git-metrics">
            <span>
              <b>{task.git_status.ahead}</b> ahead
            </span>
            <span>
              <b>{task.git_status.staged}</b> staged
            </span>
            <span>
              <b>{task.git_status.unstaged}</b> unstaged
            </span>
            <span>
              <b>{task.git_status.untracked}</b> untracked
            </span>
          </div>
          {task.git_status.diffstat ? <code>{task.git_status.diffstat}</code> : null}
        </section>
      ) : null}
      <Tabs defaultValue="instruction" className="kb-detail-tabs">
        <TabsList variant="line">
          <TabsTrigger value="instruction" icon={false}>
            Instruction
          </TabsTrigger>
          <TabsTrigger value="output" icon={false}>
            Output
          </TabsTrigger>
        </TabsList>
        <TabsContent value="instruction">
          <p>{task.instruction}</p>
        </TabsContent>
        <TabsContent value="output">
          {output ? <pre data-error={Boolean(task.error)}>{output}</pre> : <p>No output yet.</p>}
        </TabsContent>
      </Tabs>
      <div className="kb-inspector-actions">
        <div>
          {task.external_session_id ? (
            <Button size="sm" variant="ghost" onClick={() => openSession(task.external_session_id)}>
              Open session
            </Button>
          ) : null}
          {task.worktree_path || task.cwd ? (
            <Button size="sm" variant="ghost" onClick={openShell}>
              Open shell
            </Button>
          ) : null}
        </div>
        <TaskActions task={task} run={run} executor={executor} retryModel={retryModel} busy={busy} onAct={onAct} />
      </div>
    </aside>
  )
}

export function KanbanPage({ host, panelSide, onRequestClose, workingDir, conversationId, commands, setDirty }: Props) {
  const [board, setBoard] = useState<Board | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [showLaunch, setShowLaunch] = useState(false)
  const [launchError, setLaunchError] = useState<string | null>(null)
  const [launchKey, setLaunchKey] = useState(0)
  const [activeRunId, setActiveRunId] = useState<string | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)

  const refresh = useCallback(
    async (visible = true) => {
      const mine = ++generation.current
      if (visible) setRefreshing(true)
      try {
        const nextBoard = await host.iii.trigger<Board>('kanban::board', {}, { timeoutMs: 20_000 })
        if (mine !== generation.current) return
        setBoard(nextBoard)
        setActiveRunId((current) =>
          current && nextBoard.runs.some((run) => run.id === current)
            ? current
            : (nextBoard.runs.find((run) => run.status !== 'done')?.id ?? nextBoard.runs[0]?.id ?? null),
        )
        setError(null)
      } catch (cause) {
        if (mine === generation.current) setError(describe(cause))
      } finally {
        if (mine === generation.current) {
          setLoading(false)
          if (visible) setRefreshing(false)
        }
      }
    },
    [host],
  )

  useEffect(() => {
    void refresh()
  }, [refresh])
  useEffect(() => {
    setDirty?.(showLaunch ? 'New Kanban run' : false)
  }, [setDirty, showLaunch])
  useEffect(() => {
    let timer: number | null = null
    const offHandler = host.iii.on<ChangedEvent>(EVENTS_FN, () => {
      if (timer !== null) window.clearTimeout(timer)
      timer = window.setTimeout(() => void refresh(false), 80)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'kanban::changed',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      if (timer !== null) window.clearTimeout(timer)
      offTrigger()
      offHandler()
    }
  }, [host, refresh])
  useEffect(
    () =>
      commands?.register([
        {
          id: 'new-run',
          title: 'New run',
          detail: 'Create a repository-aware multi-worker run',
          shortcut: 'Mod+Enter',
          run: () => setShowLaunch(true),
        },
        {
          id: 'refresh',
          title: 'Refresh',
          detail: 'Reload sessions, worktrees, Git status, and executors',
          run: () => void refresh(),
        },
      ]),
    [commands, refresh],
  )

  const executors = board?.executors ?? []
  const models = board?.models ?? []
  const defaultModel = host.chat?.composerModel?.(conversationId) ?? models[0]?.id ?? null
  const capabilities = board?.capabilities ?? { harness: false, worktree: false, external_executors: 0 }
  const activeRun = board?.runs.find((run) => run.id === activeRunId) ?? board?.runs[0] ?? null
  const runTasks = useMemo(
    () => (board?.tasks ?? []).filter((task) => activeRun && task.run_id === activeRun.id),
    [board, activeRun],
  )
  const selected = runTasks.find((task) => task.id === selectedId) ?? null
  const executorById = useMemo(() => new Map(executors.map((executor) => [executor.id, executor])), [executors])

  useEffect(() => {
    setSelectedId(null)
  }, [activeRunId])

  const act = useCallback(
    async (task: Task, action: string, input: Record<string, unknown> = {}) => {
      setBusy(task.id)
      setFeedback(null)
      try {
        await host.iii.trigger(
          `kanban::tasks::${action}`,
          { task_id: task.id, ...input },
          { timeoutMs: action === 'land' ? 35_000 : 20_000 },
        )
        const label =
          action === 'accept'
            ? 'accepted'
            : action === 'stop'
              ? 'stopped'
              : action === 'land'
                ? 'queued for landing'
                : 'started'
        setFeedback({ kind: 'success', text: `${task.title}: ${label}` })
        await refresh(false)
      } catch (cause) {
        setFeedback({ kind: 'error', text: describe(cause) })
      } finally {
        setBusy(null)
      }
    },
    [host, refresh],
  )

  const createRun = useCallback(
    async (input: Record<string, unknown>) => {
      setBusy('create')
      setFeedback(null)
      setLaunchError(null)
      try {
        const result = await host.iii.trigger<{ run: Run; tasks: Task[] }>('kanban::runs::create', input, {
          timeoutMs: 60_000,
        })
        setShowLaunch(false)
        setLaunchError(null)
        setLaunchKey((current) => current + 1)
        setActiveRunId(result.run.id)
        setFeedback({ kind: 'success', text: `${result.run.title}: ${result.tasks.length} tasks created` })
        await refresh(false)
      } catch (cause) {
        setFeedback({ kind: 'error', text: describe(cause) })
        setLaunchError(describe(cause))
      } finally {
        setBusy(null)
      }
    },
    [host, refresh],
  )

  return (
    <PageShell className="kb-shell">
      <PageHeader
        title="Kanban"
        description={activeRun ? `${activeRun.title} · ${runTasks.length} tasks` : 'Multi-worker runs'}
        actions={
          <div className="kb-header-actions">
            <Button variant="ghost" size="sm" disabled={refreshing} onClick={() => void refresh()}>
              {refreshing ? 'Refreshing…' : 'Refresh'}
            </Button>
            <Button variant="primary" size="sm" onClick={() => setShowLaunch(true)}>
              New run
            </Button>
          </div>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="Runs"
          side={panelSide}
          header={
            <div className="kb-sidebar-title">
              <strong>Runs</strong>
              <span>{board?.runs.length ?? 0}</span>
            </div>
          }
          defaultWidth={248}
          minWidth={208}
          maxWidth={360}
          collapsible
          resizable
          storageKey="kanban-runs-sidebar"
          narrowBelow={720}
        >
          <RunSidebar runs={board?.runs ?? []} activeRunId={activeRun?.id ?? null} onSelect={setActiveRunId} />
        </PageSidebar>
        <PageMain className="kb-main">
          {error ? <StatusPanel variant="alert" headline="Kanban worker unavailable" detail={error} /> : null}
          {feedback ? (
            <StatusPanel variant={feedback.kind === 'error' ? 'alert' : 'success'} headline={feedback.text} />
          ) : null}
          {loading && !board ? (
            <div className="kb-loading">
              <Skeleton />
              <Skeleton />
              <Skeleton />
            </div>
          ) : null}
          {!loading && board && board.runs.length === 0 ? (
            <EmptyState
              title="No runs yet"
              description="Create a run that connects a root session to child executors and optional isolated Git worktrees."
              action={{ label: 'New run', onClick: () => setShowLaunch(true) }}
            />
          ) : null}
          {activeRun ? (
            <div className="kb-run-workspace">
              <section className="kb-run-toolbar">
                <div className="kb-run-heading">
                  <div>
                    <Badge variant={statusBadge(activeRun.status)}>{activeRun.status}</Badge>
                    <h1>{activeRun.title}</h1>
                  </div>
                  <p>{activeRun.goal || activeRun.repo_path || 'Harness session tree'}</p>
                </div>
                <div className="kb-run-actions">
                  {activeRun.root_session_id ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => host.chat?.selectConversation?.(activeRun.root_session_id as string)}
                    >
                      Open root
                    </Button>
                  ) : null}
                  {capabilities.worktree ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => host.panels?.open({ pageId: 'worktree', context: {} })}
                    >
                      Worktrees
                    </Button>
                  ) : null}
                </div>
              </section>
              <section className="kb-topology" aria-label="Run topology">
                <div>
                  <span>ROOT</span>
                  <strong>Harness session</strong>
                </div>
                <i>→</i>
                <div>
                  <span>EXECUTORS</span>
                  <strong>
                    {capabilities.harness ? 'Harness' : 'No Harness'} · {capabilities.external_executors} external
                  </strong>
                </div>
                <i>→</i>
                <div>
                  <span>WORKSPACE</span>
                  <strong>
                    {activeRun.isolation === 'worktree_per_task' ? 'Worktree per task' : 'Shared directory'}
                  </strong>
                </div>
                <i>→</i>
                <div>
                  <span>GIT</span>
                  <strong>{activeRun.target_branch ? `Review → ${activeRun.target_branch}` : 'Review only'}</strong>
                </div>
              </section>
              <div className="kb-workbench" data-selected={selected ? '' : undefined}>
                <KanbanBoard
                  tasks={runTasks}
                  allTasks={board?.tasks ?? []}
                  selectedId={selectedId}
                  executors={executorById}
                  onSelect={setSelectedId}
                />
                {selected ? (
                  <Inspector
                    host={host}
                    run={activeRun}
                    task={selected}
                    allTasks={board?.tasks ?? []}
                    executor={executorById.get(selected.executor_id)}
                    models={models}
                    defaultModel={defaultModel}
                    busy={busy === selected.id}
                    onBack={() => setSelectedId(null)}
                    onAct={(action, input) => void act(selected, action, input)}
                  />
                ) : null}
              </div>
            </div>
          ) : null}
        </PageMain>
      </PageBody>
      <NewRunDialog
        key={launchKey}
        open={showLaunch}
        executors={executors}
        models={models}
        defaultModel={defaultModel}
        capabilities={capabilities}
        initialRepo={workingDir?.trim() || activeRun?.repo_path || board?.runs.find((run) => run.repo_path)?.repo_path}
        recentDirectories={host.workspace?.recentDirectories() ?? []}
        busy={busy === 'create'}
        error={launchError}
        onOpenChange={(open) => {
          if (!open) setLaunchError(null)
          setShowLaunch(open)
        }}
        onCreate={createRun}
      />
    </PageShell>
  )
}

export default function setup(host: Host) {
  host.pages.register({
    id: 'kanban',
    title: 'Kanban',
    render: (props: PageRenderProps) => <KanbanPage host={host} {...props} />,
  })
  host.commands?.register('kanban', [
    {
      id: 'open',
      title: 'Open Kanban',
      detail: 'Repository-aware multi-worker runs, worktrees, Git review, and landing',
      keywords: ['runs', 'tasks', 'workers', 'worktrees', 'git'],
      run: () => host.panels?.open({ pageId: 'kanban', context: {} }),
    },
  ])
}
