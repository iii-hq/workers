export const TASK_STATUSES = ['needs_you', 'queued', 'running', 'review', 'ready', 'done'] as const

export type TaskStatus = (typeof TASK_STATUSES)[number]
export type RunStatus = 'needs_you' | 'active' | 'review' | 'ready' | 'done'

export type ExecutorKind = 'task' | 'harness'
export type IsolationMode = 'shared' | 'worktree_per_task'

export interface GitStatus {
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
  integration_reason?: string
}

export interface Executor {
  id: string
  label: string
  kind: ExecutorKind
  function_id: string
  worker_name: string
  namespace?: string
  stop_function?: string
  status_function?: string
  available: boolean
}

export interface FunctionCatalogRow {
  function_id?: string
  worker_name?: string
  namespace?: string
  description?: string | null
}

export interface RuntimeCapabilities {
  harness: boolean
  worktree: boolean
  external_executors: number
}

export interface RunRecord {
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
  created_at_ms: number
  updated_at_ms: number
}

export interface TaskRecord {
  id: string
  key?: string
  run_id: string
  title: string
  instruction: string
  executor_id: string
  executor_function: string
  executor_kind: ExecutorKind
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
  status: TaskStatus
  attempt: number
  external_session_id?: string
  external_turn_id?: string
  result?: unknown
  error?: string
  created_at_ms: number
  updated_at_ms: number
}

export interface CreateTaskInput {
  key: string
  title?: string
  instruction: string
  executor: string
  depends_on: string[]
  cwd?: string
  model?: string
  profile?: string
}

export interface CreateRunInput {
  title?: string
  goal?: string
  cwd?: string
  repo_path?: string
  base_ref?: string
  target_branch?: string
  isolation: IsolationMode
  auto_dispatch?: boolean
  tasks: CreateTaskInput[]
}

export interface RunView extends RunRecord {
  source: 'kanban' | 'harness'
  status: RunStatus
  counts: Record<TaskStatus, number>
}

export interface TaskView extends TaskRecord {
  source: 'kanban' | 'harness'
  run_title: string
  git_status?: GitStatus
}

export function isTaskStatus(value: unknown): value is TaskStatus {
  return typeof value === 'string' && (TASK_STATUSES as readonly string[]).includes(value)
}

export function taskCounts(tasks: TaskRecord[]): Record<TaskStatus, number> {
  const counts = Object.fromEntries(TASK_STATUSES.map((status) => [status, 0])) as Record<TaskStatus, number>
  for (const task of tasks) counts[task.status] += 1
  return counts
}

export function deriveRunStatus(tasks: TaskRecord[]): RunStatus {
  if (tasks.some((task) => task.status === 'needs_you')) return 'needs_you'
  if (tasks.some((task) => task.status === 'queued' || task.status === 'running')) return 'active'
  if (tasks.some((task) => task.status === 'review')) return 'review'
  if (tasks.some((task) => task.status === 'ready')) return 'ready'
  return 'done'
}

export function dependenciesSatisfied(task: TaskRecord, tasks: TaskRecord[]): boolean {
  if (!task.depends_on?.length) return true
  const byId = new Map(tasks.map((candidate) => [candidate.id, candidate]))
  return task.depends_on.every((id) => {
    const dependency = byId.get(id)
    return dependency?.status === 'review' || dependency?.status === 'done'
  })
}

export function singleFlight<K, V>(inFlight: Map<K, Promise<V>>, key: K, work: () => Promise<V>): Promise<V> {
  const existing = inFlight.get(key)
  if (existing) return existing
  const pending = Promise.resolve()
    .then(work)
    .finally(() => {
      if (inFlight.get(key) === pending) inFlight.delete(key)
    })
  inFlight.set(key, pending)
  return pending
}

export function validateCreateRun(input: unknown): CreateRunInput {
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('INVALID_RUN: expected an object')
  }
  const raw = input as Record<string, unknown>
  if (!Array.isArray(raw.tasks) || raw.tasks.length === 0) {
    throw new Error('INVALID_RUN: tasks must contain at least one task')
  }
  if (raw.tasks.length > 24) throw new Error('INVALID_RUN: a run supports at most 24 tasks')
  const tasks = raw.tasks.map((value, index): CreateTaskInput => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      throw new Error(`INVALID_TASK: tasks[${index}] must be an object`)
    }
    const task = value as Record<string, unknown>
    const instruction = text(task.instruction)
    const executor = text(task.executor)
    if (!instruction) throw new Error(`INVALID_TASK: tasks[${index}].instruction is required`)
    if (!executor) throw new Error(`INVALID_TASK: tasks[${index}].executor is required`)
    return {
      key: text(task.key) ?? `task-${index + 1}`,
      instruction,
      executor,
      depends_on: Array.isArray(task.depends_on)
        ? [...new Set(task.depends_on.map(text).filter((key): key is string => Boolean(key)))]
        : [],
      ...(text(task.title) ? { title: text(task.title) } : {}),
      ...(text(task.cwd) ? { cwd: text(task.cwd) } : {}),
      ...(text(task.model) ? { model: text(task.model) } : {}),
      ...(text(task.profile) ? { profile: text(task.profile) } : {}),
    }
  })
  const keys = new Set<string>()
  for (const [index, task] of tasks.entries()) {
    if (keys.has(task.key)) throw new Error(`INVALID_TASK: tasks[${index}].key must be unique`)
    keys.add(task.key)
  }
  for (const [index, task] of tasks.entries()) {
    if (task.depends_on.includes(task.key)) {
      throw new Error(`INVALID_TASK: tasks[${index}] cannot depend on itself`)
    }
    const missing = task.depends_on.find((key) => !keys.has(key))
    if (missing) throw new Error(`INVALID_TASK: tasks[${index}].depends_on references unknown key ${missing}`)
  }
  const taskByKey = new Map(tasks.map((task) => [task.key, task]))
  const visiting = new Set<string>()
  const visited = new Set<string>()
  const visit = (key: string) => {
    if (visiting.has(key)) throw new Error(`INVALID_TASK: dependency cycle includes ${key}`)
    if (visited.has(key)) return
    visiting.add(key)
    for (const dependency of taskByKey.get(key)?.depends_on ?? []) visit(dependency)
    visiting.delete(key)
    visited.add(key)
  }
  for (const task of tasks) visit(task.key)
  const isolation: IsolationMode = raw.isolation === 'worktree_per_task' ? 'worktree_per_task' : 'shared'
  const repoPath = text(raw.repo_path) ?? text(raw.cwd)
  if (isolation === 'worktree_per_task' && !repoPath) {
    throw new Error('INVALID_RUN: repo_path is required for per-task worktrees')
  }
  return {
    tasks,
    isolation,
    auto_dispatch: raw.auto_dispatch !== false,
    ...(text(raw.title) ? { title: text(raw.title) } : {}),
    ...(text(raw.goal) ? { goal: text(raw.goal) } : {}),
    ...(text(raw.cwd) ? { cwd: text(raw.cwd) } : {}),
    ...(repoPath ? { repo_path: repoPath } : {}),
    ...(text(raw.base_ref) ? { base_ref: text(raw.base_ref) } : {}),
    ...(text(raw.target_branch) ? { target_branch: text(raw.target_branch) } : {}),
  }
}

function text(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined
  const trimmed = value.trim()
  return trimmed || undefined
}

export function executorLabel(functionId: string, workerName: string): string {
  const base = workerName || functionId.split('::')[0] || functionId
  return base
    .split(/[-_]/g)
    .filter(Boolean)
    .map((part) => part[0]?.toUpperCase() + part.slice(1))
    .join(' ')
}

function catalogHas(catalog: FunctionCatalogRow[], functionId: string, namespace?: string): boolean {
  return catalog.some((row) => row.function_id === functionId && row.namespace === namespace)
}

export function executorsFromCatalog(catalog: FunctionCatalogRow[]): Executor[] {
  const executors: Executor[] = []

  for (const row of catalog) {
    const functionId = row.function_id
    if (!functionId?.endsWith('::task')) continue
    if (!row.description?.includes('agent_tasks')) continue
    const prefix = functionId.slice(0, -'::task'.length)
    executors.push({
      id: row.namespace ? `${prefix}@${row.namespace}` : prefix,
      label: executorLabel(functionId, row.worker_name ?? prefix),
      kind: 'task',
      function_id: functionId,
      worker_name: row.worker_name ?? prefix,
      ...(row.namespace ? { namespace: row.namespace } : {}),
      ...(catalogHas(catalog, `${prefix}::stop`, row.namespace) ? { stop_function: `${prefix}::stop` } : {}),
      ...(catalogHas(catalog, `${prefix}::status`, row.namespace) ? { status_function: `${prefix}::status` } : {}),
      available: true,
    })
  }

  for (const harness of catalog.filter((row) => row.function_id === 'harness::spawn')) {
    executors.push({
      id: harness.namespace ? `harness@${harness.namespace}` : 'harness',
      label: 'Harness',
      kind: 'harness',
      function_id: 'harness::spawn',
      worker_name: harness.worker_name ?? 'harness',
      ...(harness.namespace ? { namespace: harness.namespace } : {}),
      ...(catalogHas(catalog, 'harness::stop', harness.namespace) ? { stop_function: 'harness::stop' } : {}),
      ...(catalogHas(catalog, 'harness::status', harness.namespace) ? { status_function: 'harness::status' } : {}),
      available: true,
    })
  }

  return executors.sort((a, b) => a.label.localeCompare(b.label))
}

export function runtimeCapabilities(catalog: FunctionCatalogRow[], executors: Executor[]): RuntimeCapabilities {
  const ids = new Set(catalog.map((row) => row.function_id))
  return {
    harness: ids.has('harness::spawn'),
    worktree: ids.has('worktree::create') && ids.has('worktree::status') && ids.has('worktree::land'),
    external_executors: executors.filter((executor) => executor.kind === 'task').length,
  }
}

export function resultText(value: unknown): string | undefined {
  if (value === undefined || value === null) return undefined
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}
