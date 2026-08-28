import { randomUUID } from 'node:crypto'
import { watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { uiPage, uiStyles } from 'virtual:kanban-ui-assets'
import { registerWorker } from 'iii-sdk'
import {
  dependenciesSatisfied,
  deriveRunStatus,
  type Executor,
  executorsFromCatalog,
  type FunctionCatalogRow,
  type GitStatus,
  isTaskStatus,
  type RunRecord,
  type RunView,
  resultText,
  runtimeCapabilities,
  singleFlight,
  type TaskRecord,
  type TaskView,
  taskCounts,
  validateCreateRun,
} from './model.js'

const WORKER = 'kanban'
const RUNS_SCOPE = 'kanban_runs'
const TASKS_SCOPE = 'kanban_tasks'
const AGENT_TASKS_SCOPE = 'agent_tasks'
const CHANGED_TRIGGER = 'kanban::changed'
const TASK_OUTCOME_FN = 'kanban::on-agent-task'
const HARNESS_COMPLETED_FN = 'kanban::on-harness-completed'
const SESSION_CHANGED_FN = 'kanban::on-session-changed'
const WORKTREE_CHANGED_FN = 'kanban::on-worktree-changed'

const iii = registerWorker(process.env.III_URL ?? process.env.III_ENGINE_URL, {
  workerName: WORKER,
  workerDescription: 'Repository-aware multi-worker runs with Harness sessions, isolated worktrees, and Git review.',
})

const object = (properties: Record<string, unknown> = {}, required: string[] = []) => ({
  type: 'object' as const,
  properties,
  ...(required.length ? { required } : {}),
})
const string = { type: 'string' }
const nullableString = { type: ['string', 'null'] }

type FunctionList = { functions?: FunctionCatalogRow[] }

type SessionRecord = {
  session_id: string
  title?: string
  description?: string
  status?: string
  status_reason?: string
  message_count?: number
  metadata?: Record<string, unknown>
  created_at?: number
  updated_at?: number
}

type SessionList = { sessions?: SessionRecord[]; next_cursor?: string | null }

type WorktreeCreateResponse = {
  worktree_id: string
  path: string
  branch: string
  base_ref: string
  base_sha: string
  dev_port: number
}

type WorktreeStatusResponse = GitStatus & {
  worktree_id: string
  branch: string
  lifecycle: 'active' | 'claimed' | 'landing' | 'land-blocked' | 'orphaned'
  dev_port: number
}

type WorktreeLandResponse = { job_id: string; queued: boolean }

type CatalogModel = {
  id: string
  provider: string
  display_name?: string
  supports_tools?: boolean
}

type ModelOption = { id: string; label: string; provider: string }

let modelCache: { expiresAt: number; models: ModelOption[] } | null = null
const activeDispatches = new Map<string, Promise<TaskRecord>>()

async function listFunctionRows(search?: string): Promise<FunctionCatalogRow[]> {
  const response = await iii.trigger<Record<string, unknown>, FunctionList>({
    function_id: 'engine::functions::list',
    payload: { include_internal: false, ...(search ? { search } : {}) },
    timeoutMs: 10_000,
  })
  return Array.isArray(response?.functions) ? response.functions : []
}

async function discoverExecutors(): Promise<Executor[]> {
  return executorsFromCatalog(await listFunctionRows())
}

async function triggerCatalogFunction<I extends Record<string, unknown>, O>(
  catalog: FunctionCatalogRow[],
  functionId: string,
  payload: I,
  timeoutMs = 15_000,
): Promise<O> {
  const row = catalog.find((candidate) => candidate.function_id === functionId)
  if (!row) throw new Error(`FUNCTION_UNAVAILABLE: ${functionId}`)
  return iii.trigger<I, O>({
    function_id: functionId,
    payload,
    timeoutMs,
    ...(row.namespace ? { namespace: row.namespace } : {}),
  })
}

async function discoverModels(catalog: FunctionCatalogRow[]): Promise<ModelOption[]> {
  if (modelCache && modelCache.expiresAt > Date.now()) return modelCache.models
  if (!catalog.some((row) => row.function_id === 'router::models::list')) return []
  const response = await triggerCatalogFunction<Record<string, unknown>, { models?: CatalogModel[] }>(
    catalog,
    'router::models::list',
    {},
  )
  const models = (response.models ?? [])
    .filter((model) => model.supports_tools !== false && model.provider && model.id)
    .map((model) => ({
      id: `${model.provider}::${model.id}`,
      label: model.display_name?.trim() || model.id,
      provider: model.provider,
    }))
  modelCache = { expiresAt: Date.now() + 30_000, models }
  return models
}

async function stateGet<T>(scope: string, key: string): Promise<T | null> {
  const value = await iii.trigger<Record<string, unknown>, T | null>({
    function_id: 'state::get',
    payload: { scope, key },
    timeoutMs: 10_000,
  })
  return value ?? null
}

async function stateList<T>(scope: string): Promise<T[]> {
  const value = await iii.trigger<Record<string, unknown>, T[] | null>({
    function_id: 'state::list',
    payload: { scope },
    timeoutMs: 10_000,
  })
  return Array.isArray(value) ? value : []
}

async function stateSet<T>(scope: string, key: string, value: T): Promise<void> {
  await iii.trigger({
    function_id: 'state::set',
    payload: { scope, key, value },
    timeoutMs: 10_000,
  })
}

type ChangeBinding = { id: string; function_id: string; namespace?: string }
const changeBindings = new Map<string, ChangeBinding>()

iii.registerTriggerType<Record<string, never>>(
  {
    id: CHANGED_TRIGGER,
    description: 'Fires when a Kanban run or task changes. Bind with an empty config.',
  },
  {
    async registerTrigger({ id, function_id, namespace }) {
      changeBindings.set(id, { id, function_id, namespace })
    },
    async unregisterTrigger({ id }) {
      changeBindings.delete(id)
    },
  },
)

async function emitChanged(kind: 'run' | 'task', id: string, runId: string): Promise<void> {
  const payload = { kind, id, run_id: runId, updated_at_ms: Date.now() }
  await Promise.all(
    [...changeBindings.values()].map((binding) =>
      iii
        .trigger({
          function_id: binding.function_id,
          payload,
          timeoutMs: 10_000,
          ...(binding.namespace ? { namespace: binding.namespace } : {}),
        })
        .catch((error) =>
          console.error(`[${WORKER}] change delivery to ${binding.function_id} failed: ${String(error)}`),
        ),
    ),
  )
}

async function saveRun(run: RunRecord): Promise<void> {
  await stateSet(RUNS_SCOPE, run.id, run)
  await emitChanged('run', run.id, run.id)
}

async function saveTask(task: TaskRecord): Promise<void> {
  await stateSet(TASKS_SCOPE, task.id, task)
  await emitChanged('task', task.id, task.run_id)
}

function validRun(value: unknown): value is RunRecord {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const run = value as Partial<RunRecord>
  return typeof run.id === 'string' && typeof run.title === 'string' && Array.isArray(run.task_ids)
}

function validTask(value: unknown): value is TaskRecord {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const task = value as Partial<TaskRecord>
  return (
    typeof task.id === 'string' &&
    typeof task.run_id === 'string' &&
    typeof task.title === 'string' &&
    typeof task.instruction === 'string' &&
    isTaskStatus(task.status)
  )
}

async function loadTask(id: string): Promise<TaskRecord> {
  const task = await stateGet<TaskRecord>(TASKS_SCOPE, id)
  if (!validTask(task)) throw new Error(`TASK_NOT_FOUND: ${id}`)
  return task
}

function sessionParent(session: SessionRecord): string | undefined {
  const value = session.metadata?.parent_session_id
  return typeof value === 'string' && value ? value : undefined
}

function sessionTitle(session: SessionRecord): string {
  const display = session.metadata?.subagent_display
  if (display && typeof display === 'object' && !Array.isArray(display)) {
    const name = (display as Record<string, unknown>).name
    if (typeof name === 'string' && name.trim()) return name.trim()
  }
  return session.title?.trim() || session.session_id
}

function sessionTaskStatus(status?: string): TaskRecord['status'] {
  if (status === 'working') return 'running'
  if (status === 'done') return 'done'
  if (status === 'error') return 'needs_you'
  if (status === 'idle') return 'ready'
  return 'queued'
}

function sessionFilesystemRoot(session: SessionRecord): string | undefined {
  const scope = session.metadata?.fs_scope
  if (!scope || typeof scope !== 'object' || Array.isArray(scope)) return undefined
  const root = (scope as Record<string, unknown>).root
  return typeof root === 'string' && root ? root : undefined
}

function rootSession(session: SessionRecord, byId: Map<string, SessionRecord>): SessionRecord {
  let current = session
  const seen = new Set([current.session_id])
  while (true) {
    const parentId = sessionParent(current)
    if (!parentId || seen.has(parentId)) return current
    const parent = byId.get(parentId)
    if (!parent) return current
    seen.add(parentId)
    current = parent
  }
}

async function board() {
  const [rawRuns, rawTasks, rawSessions, catalog] = await Promise.all([
    stateList<RunRecord>(RUNS_SCOPE),
    stateList<TaskRecord>(TASKS_SCOPE),
    iii.trigger<Record<string, unknown>, SessionList>({
      function_id: 'session::list',
      payload: { limit: 200, order: 'updated_desc' },
      timeoutMs: 10_000,
    }),
    listFunctionRows(),
  ])
  const executors = executorsFromCatalog(catalog)
  const models = await discoverModels(catalog)
  const runs = rawRuns.filter(validRun)
  const tasks = rawTasks.filter(validTask)
  const runById = new Map(runs.map((run) => [run.id, run]))
  const runViews: RunView[] = runs.map((run) => ({
    ...run,
    source: 'kanban',
    status: 'ready',
    counts: taskCounts([]),
  }))
  const taskViews: TaskView[] = tasks.map((task) => ({
    ...task,
    source: 'kanban',
    run_title: runById.get(task.run_id)?.title ?? task.run_id,
  }))

  if (catalog.some((row) => row.function_id === 'worktree::status')) {
    await Promise.all(
      taskViews.map(async (task) => {
        if (!task.worktree_id) return
        try {
          const status = await triggerCatalogFunction<Record<string, unknown>, WorktreeStatusResponse>(
            catalog,
            'worktree::status',
            { worktree_id: task.worktree_id },
          )
          task.git_status = status
          task.dev_port = status.dev_port
          if (status.lifecycle === 'land-blocked') task.land_status = 'blocked'
        } catch {}
      }),
    )
  }

  const sessions = Array.isArray(rawSessions?.sessions) ? rawSessions.sessions : []
  const sessionById = new Map(sessions.map((session) => [session.session_id, session]))
  const ownedExternalSessions = new Set(tasks.map((task) => task.external_session_id).filter(Boolean))
  const ownedRunByRoot = new Map(
    runs.filter((run) => run.root_session_id).map((run) => [run.root_session_id as string, run]),
  )
  const projectedRunByRoot = new Map<string, RunView>()

  for (const session of sessions) {
    if (!sessionParent(session) || ownedExternalSessions.has(session.session_id)) continue
    const root = rootSession(session, sessionById)
    const ownedRun = ownedRunByRoot.get(root.session_id)
    const runId = ownedRun?.id ?? `harness:${root.session_id}`
    const runTitle = ownedRun?.title ?? sessionTitle(root)
    const status = sessionTaskStatus(session.status)
    const instruction =
      session.status_reason?.trim() ||
      session.description?.trim() ||
      `Harness session with ${session.message_count ?? 0} messages`
    taskViews.push({
      id: `harness:${session.session_id}`,
      run_id: runId,
      run_title: runTitle,
      title: sessionTitle(session),
      instruction,
      executor_id: 'harness',
      executor_function: 'harness::spawn',
      executor_kind: 'harness',
      status,
      attempt: 1,
      external_session_id: session.session_id,
      ...(sessionFilesystemRoot(session) ? { cwd: sessionFilesystemRoot(session) } : {}),
      ...(status === 'needs_you' ? { error: session.status_reason || 'Harness session failed' } : {}),
      created_at_ms: session.created_at ?? Date.now(),
      updated_at_ms: session.updated_at ?? session.created_at ?? Date.now(),
      source: 'harness',
    })
    if (!ownedRun && !projectedRunByRoot.has(root.session_id)) {
      const projected: RunView = {
        id: runId,
        title: runTitle,
        ...(root.description?.trim() ? { goal: root.description.trim() } : {}),
        root_session_id: root.session_id,
        task_ids: [],
        created_at_ms: root.created_at ?? Date.now(),
        updated_at_ms: root.updated_at ?? root.created_at ?? Date.now(),
        source: 'harness',
        status: 'ready',
        counts: taskCounts([]),
      }
      projectedRunByRoot.set(root.session_id, projected)
      runViews.push(projected)
    }
  }

  for (const run of runViews) {
    const children = taskViews.filter((task) => task.run_id === run.id)
    run.task_ids = children.map((task) => task.id)
    run.status = deriveRunStatus(children)
    run.counts = taskCounts(children)
    run.updated_at_ms = Math.max(run.updated_at_ms, ...children.map((task) => task.updated_at_ms))
  }
  runViews.sort((a, b) => b.updated_at_ms - a.updated_at_ms)
  taskViews.sort((a, b) => b.updated_at_ms - a.updated_at_ms)
  return {
    runs: runViews,
    tasks: taskViews,
    executors,
    models,
    capabilities: runtimeCapabilities(catalog, executors),
    updated_at_ms: Date.now(),
  }
}

function dispatchTask(taskId: string, retry = false): Promise<TaskRecord> {
  return singleFlight(activeDispatches, taskId, () => dispatchTaskOnce(taskId, retry))
}

async function dispatchTaskOnce(taskId: string, retry = false): Promise<TaskRecord> {
  const task = await loadTask(taskId)
  if (!['ready', 'queued', 'needs_you'].includes(task.status)) {
    throw new Error(`TASK_NOT_DISPATCHABLE: ${task.id} is ${task.status}`)
  }
  const executor = (await discoverExecutors()).find((candidate) => candidate.id === task.executor_id)
  if (!executor) throw new Error(`EXECUTOR_UNAVAILABLE: ${task.executor_id}`)
  const run = await stateGet<RunRecord>(RUNS_SCOPE, task.run_id)
  const parentSessionId = validRun(run) ? run.root_session_id : undefined
  const runTasks = (await stateList<TaskRecord>(TASKS_SCOPE)).filter(
    (candidate): candidate is TaskRecord => validTask(candidate) && candidate.run_id === task.run_id,
  )
  if (!dependenciesSatisfied(task, runTasks)) {
    throw new Error(`TASK_DEPENDENCIES_PENDING: ${task.id}`)
  }

  const sessionId = (retry && !task.worktree_id) || !task.external_session_id ? randomUUID() : task.external_session_id
  let next: TaskRecord = {
    ...task,
    status: 'running',
    executor_function: executor.function_id,
    executor_kind: executor.kind,
    external_session_id: sessionId,
    external_turn_id: undefined,
    result: undefined,
    error: undefined,
    attempt: retry ? task.attempt + 1 : task.attempt,
    updated_at_ms: Date.now(),
  }
  await saveTask(next)

  try {
    if (executor.kind === 'harness') {
      const response = await iii.trigger<
        Record<string, unknown>,
        { child_session_id?: string; child_turn_id?: string }
      >({
        function_id: executor.function_id,
        payload: {
          task: task.instruction,
          session_id: sessionId,
          ...(parentSessionId ? { parent_session_id: parentSessionId } : {}),
          display: { name: task.title, icon: 'code', color: 'blue' },
          ...(task.worktree_path ? { options: { filesystem_root: task.worktree_path } } : {}),
          ...(task.model ? { model: task.model } : {}),
          ...(task.profile ? { agent: task.profile } : {}),
        },
        timeoutMs: 15_000,
        ...(executor.namespace ? { namespace: executor.namespace } : {}),
      })
      next = {
        ...next,
        external_session_id: response?.child_session_id ?? sessionId,
        ...(response?.child_turn_id ? { external_turn_id: response.child_turn_id } : {}),
        updated_at_ms: Date.now(),
      }
    } else {
      const response = await iii.trigger<
        Record<string, unknown>,
        { session_id?: string; started?: boolean; reason?: string }
      >({
        function_id: executor.function_id,
        payload: {
          task: task.instruction,
          session_id: sessionId,
          ...(parentSessionId ? { parent_session_id: parentSessionId } : {}),
          ...(task.worktree_path || task.cwd ? { cwd: task.worktree_path ?? task.cwd } : {}),
          ...(task.model ? { model: task.model } : {}),
        },
        timeoutMs: 15_000,
        ...(executor.namespace ? { namespace: executor.namespace } : {}),
      })
      if (response?.started === false) throw new Error(response.reason || 'executor did not start the task')
      next = {
        ...next,
        external_session_id: response?.session_id ?? sessionId,
        updated_at_ms: Date.now(),
      }
    }
    await saveTask(next)
    return next
  } catch (error) {
    next = {
      ...next,
      status: 'needs_you',
      error: error instanceof Error ? error.message : String(error),
      updated_at_ms: Date.now(),
    }
    await saveTask(next)
    return next
  }
}

async function releaseDependents(runId: string): Promise<void> {
  const tasks = (await stateList<TaskRecord>(TASKS_SCOPE)).filter(
    (task): task is TaskRecord => validTask(task) && task.run_id === runId,
  )
  const eligible = tasks.filter((task) => task.status === 'queued' && dependenciesSatisfied(task, tasks))
  await Promise.all(eligible.map((task) => dispatchTask(task.id)))
}

async function prepareTaskWorktree(
  task: TaskRecord,
  run: RunRecord,
  catalog: FunctionCatalogRow[],
): Promise<TaskRecord> {
  if (run.isolation !== 'worktree_per_task') return task
  if (!run.repo_path) throw new Error('RUN_REPOSITORY_MISSING')
  try {
    const created = await triggerCatalogFunction<Record<string, unknown>, WorktreeCreateResponse>(
      catalog,
      'worktree::create',
      {
        repo_path: run.repo_path,
        ...(run.base_ref ? { base_ref: run.base_ref } : {}),
        ...(task.external_session_id ? { session_id: task.external_session_id } : {}),
      },
      30_000,
    )
    return {
      ...task,
      cwd: created.path,
      worktree_id: created.worktree_id,
      worktree_path: created.path,
      branch: created.branch,
      base_ref: created.base_ref,
      base_sha: created.base_sha,
      dev_port: created.dev_port,
      updated_at_ms: Date.now(),
    }
  } catch (error) {
    return {
      ...task,
      status: 'needs_you',
      error: `Worktree setup failed: ${error instanceof Error ? error.message : String(error)}`,
      updated_at_ms: Date.now(),
    }
  }
}

iii.registerFunction(
  TASK_OUTCOME_FN,
  async (event: { scope?: string; key?: string; new_value?: unknown }) => {
    if (event.scope !== AGENT_TASKS_SCOPE || !event.key || !event.new_value || typeof event.new_value !== 'object') {
      return { updated: false }
    }
    const outcome = event.new_value as Record<string, unknown>
    const sessionId = typeof outcome.session_id === 'string' ? outcome.session_id : event.key
    const tasks = (await stateList<TaskRecord>(TASKS_SCOPE)).filter(validTask)
    const task = tasks.find((candidate) => candidate.external_session_id === sessionId)
    if (!task || task.status === 'done') return { updated: false }
    const ok = outcome.status === 'done'
    const next: TaskRecord = {
      ...task,
      status: ok ? 'review' : 'needs_you',
      ...(ok ? { result: outcome.result } : { error: resultText(outcome.error) ?? 'Task failed' }),
      updated_at_ms: typeof outcome.updated_at_ms === 'number' ? outcome.updated_at_ms : Date.now(),
    }
    await saveTask(next)
    if (ok) await releaseDependents(next.run_id)
    return { updated: true, task_id: next.id, status: next.status }
  },
  {
    description: 'Internal completion sink for task-contract executors.',
    metadata: { internal: true },
    request_format: object(),
    response_format: object({ updated: { type: 'boolean' }, task_id: string, status: string }, ['updated']),
  },
)

iii.registerFunction(
  HARNESS_COMPLETED_FN,
  async (event: Record<string, unknown>) => {
    if (event.terminal !== true || typeof event.session_id !== 'string') return { updated: false }
    const tasks = (await stateList<TaskRecord>(TASKS_SCOPE)).filter(validTask)
    const task = tasks.find(
      (candidate) => candidate.executor_kind === 'harness' && candidate.external_session_id === event.session_id,
    )
    if (!task || task.status === 'done') return { updated: false }
    const ok = event.status === 'completed'
    const next: TaskRecord = {
      ...task,
      status: ok ? 'review' : 'needs_you',
      ...(typeof event.turn_id === 'string' ? { external_turn_id: event.turn_id } : {}),
      ...(ok
        ? { result: event.result }
        : { error: resultText(event.result_error ?? event.reason) ?? `Harness task ${String(event.status)}` }),
      updated_at_ms: typeof event.timestamp === 'number' ? event.timestamp : Date.now(),
    }
    await saveTask(next)
    if (ok) await releaseDependents(next.run_id)
    return { updated: true, task_id: next.id, status: next.status }
  },
  {
    description: 'Internal completion sink for Harness tasks.',
    metadata: { internal: true },
    request_format: object(),
    response_format: object({ updated: { type: 'boolean' }, task_id: string, status: string }, ['updated']),
  },
)

iii.registerFunction(
  SESSION_CHANGED_FN,
  async (event: Record<string, unknown>) => {
    const sessionId = typeof event.session_id === 'string' ? event.session_id : 'sessions'
    const parentSessionId = typeof event.parent_session_id === 'string' ? event.parent_session_id : sessionId
    await emitChanged('run', sessionId, parentSessionId)
    return { updated: true }
  },
  {
    description: 'Internal refresh sink for Harness session-tree changes.',
    metadata: { internal: true },
    request_format: object(),
    response_format: object({ updated: { type: 'boolean' } }, ['updated']),
  },
)

iii.registerFunction(
  WORKTREE_CHANGED_FN,
  async (event: Record<string, unknown>) => {
    if (typeof event.worktree_id !== 'string') return { updated: false }
    const tasks = (await stateList<TaskRecord>(TASKS_SCOPE)).filter(validTask)
    const task = tasks.find((candidate) => candidate.worktree_id === event.worktree_id)
    if (!task) return { updated: false }
    const next: TaskRecord = {
      ...task,
      ...(typeof event.merged_sha === 'string' ? { land_status: 'landed' as const } : {}),
      ...(typeof event.reason === 'string'
        ? { land_status: 'blocked' as const, error: `Landing blocked: ${event.reason}` }
        : {}),
      updated_at_ms: typeof event.timestamp === 'number' ? event.timestamp : Date.now(),
    }
    await saveTask(next)
    return { updated: true, task_id: next.id, land_status: next.land_status }
  },
  {
    description: 'Internal refresh sink for managed worktree landing outcomes.',
    metadata: { internal: true },
    request_format: object(),
    response_format: object({ updated: { type: 'boolean' }, task_id: string, land_status: string }, ['updated']),
  },
)

iii.registerTrigger({ type: 'state', function_id: TASK_OUTCOME_FN, config: { scope: AGENT_TASKS_SCOPE } })
iii.registerTrigger({ type: 'harness::turn-completed', function_id: HARNESS_COMPLETED_FN, config: {} })
iii.registerTrigger({ type: 'session::created', function_id: SESSION_CHANGED_FN, config: {} })
iii.registerTrigger({ type: 'session::status-changed', function_id: SESSION_CHANGED_FN, config: {} })
iii.registerTrigger({ type: 'session::meta-updated', function_id: SESSION_CHANGED_FN, config: {} })
iii.registerTrigger({ type: 'worktree::landed', function_id: WORKTREE_CHANGED_FN, config: {} })
iii.registerTrigger({ type: 'worktree::land-blocked', function_id: WORKTREE_CHANGED_FN, config: {} })

iii.registerFunction('kanban::executors::list', () => discoverExecutors(), {
  description: 'List live executors that satisfy the Kanban task contract, including Harness when available.',
  request_format: object(),
  response_format: { type: 'array', items: object() },
})

iii.registerFunction('kanban::models::list', async () => discoverModels(await listFunctionRows()), {
  description: 'List live tool-capable models usable by top-level Harness child tasks.',
  request_format: object(),
  response_format: { type: 'array', items: object() },
})

iii.registerFunction('kanban::board', () => board(), {
  description: 'Project Harness session trees, executor topology, managed worktrees, and Git status into durable runs.',
  request_format: object(),
  response_format: object(
    {
      runs: { type: 'array', items: object() },
      tasks: { type: 'array', items: object() },
      executors: { type: 'array', items: object() },
      models: { type: 'array', items: object() },
      capabilities: object(),
    },
    ['runs', 'tasks', 'executors', 'models', 'capabilities'],
  ),
})

iii.registerFunction(
  'kanban::runs::create',
  async (input: unknown) => {
    const parsed = validateCreateRun(input)
    const catalog = await listFunctionRows()
    const executors = executorsFromCatalog(catalog)
    const executorById = new Map(executors.map((executor) => [executor.id, executor]))
    for (const task of parsed.tasks) {
      const executor = executorById.get(task.executor)
      if (!executor) throw new Error(`EXECUTOR_UNAVAILABLE: ${task.executor}`)
      if (executor.kind === 'harness' && !task.model) {
        throw new Error(`HARNESS_MODEL_REQUIRED: task ${task.key} must name a live model`)
      }
    }
    if (parsed.isolation === 'worktree_per_task' && !runtimeCapabilities(catalog, executors).worktree) {
      throw new Error('WORKTREE_UNAVAILABLE: start the worktree Worker before creating an isolated run')
    }

    const now = Date.now()
    const runId = randomUUID()
    const taskIds = parsed.tasks.map(() => randomUUID())
    const taskIdByKey = new Map(parsed.tasks.map((task, index) => [task.key, taskIds[index]]))
    const tasks: TaskRecord[] = parsed.tasks.map((inputTask, index) => {
      const executor = executorById.get(inputTask.executor) as Executor
      const instruction = inputTask.instruction.trim()
      return {
        id: taskIds[index],
        key: inputTask.key,
        run_id: runId,
        title: inputTask.title?.trim() || instruction.split('\n')[0].slice(0, 72) || `Task ${index + 1}`,
        instruction,
        executor_id: executor.id,
        executor_function: executor.function_id,
        executor_kind: executor.kind,
        cwd: inputTask.cwd ?? parsed.cwd,
        depends_on: inputTask.depends_on.map((key) => taskIdByKey.get(key) as string),
        model: inputTask.model,
        profile: inputTask.profile,
        status: parsed.auto_dispatch === false ? 'ready' : 'queued',
        attempt: 1,
        external_session_id: randomUUID(),
        created_at_ms: now,
        updated_at_ms: now,
      }
    })
    const runTitle = parsed.title?.trim() || tasks[0].title
    const rootSession = await iii.trigger<Record<string, unknown>, { session_id: string }>({
      function_id: 'session::create',
      payload: {
        title: runTitle,
        description: parsed.goal ?? `${tasks.length} coordinated task${tasks.length === 1 ? '' : 's'}`,
        metadata: {
          surface: 'kanban',
          mode: 'orchestration',
          kanban_run_id: runId,
          isolation: parsed.isolation,
          ...(parsed.repo_path ? { repo_path: parsed.repo_path } : {}),
        },
      },
      timeoutMs: 10_000,
    })
    const run: RunRecord = {
      id: runId,
      title: runTitle,
      goal: parsed.goal,
      cwd: parsed.cwd,
      repo_path: parsed.repo_path,
      base_ref: parsed.base_ref,
      target_branch: parsed.target_branch,
      isolation: parsed.isolation,
      root_session_id: rootSession.session_id,
      task_ids: tasks.map((task) => task.id),
      created_at_ms: now,
      updated_at_ms: now,
    }
    const prepared = await Promise.all(tasks.map((task) => prepareTaskWorktree(task, run, catalog)))
    await Promise.all([saveRun(run), ...prepared.map(saveTask)])
    if (parsed.auto_dispatch !== false) {
      await Promise.all(
        prepared
          .filter((task) => task.status === 'queued' && !task.depends_on?.length)
          .map((task) => dispatchTask(task.id)),
      )
    }
    return { run, tasks: (await stateList<TaskRecord>(TASKS_SCOPE)).filter((task) => task.run_id === runId) }
  },
  {
    description: 'Create a durable run with one or more tasks and optionally dispatch them immediately.',
    request_format: object(
      {
        title: nullableString,
        goal: nullableString,
        cwd: nullableString,
        repo_path: nullableString,
        base_ref: nullableString,
        target_branch: nullableString,
        isolation: { type: 'string', enum: ['shared', 'worktree_per_task'] },
        auto_dispatch: { type: 'boolean' },
        tasks: {
          type: 'array',
          minItems: 1,
          maxItems: 24,
          items: object(
            {
              title: nullableString,
              instruction: string,
              executor: string,
              key: nullableString,
              depends_on: { type: 'array', items: string },
              cwd: nullableString,
              model: nullableString,
              profile: nullableString,
            },
            ['instruction', 'executor'],
          ),
        },
      },
      ['tasks'],
    ),
    response_format: object({ run: object(), tasks: { type: 'array', items: object() } }, ['run', 'tasks']),
  },
)

iii.registerFunction('kanban::tasks::dispatch', (input: { task_id: string }) => dispatchTask(input.task_id), {
  description: 'Dispatch one ready or attention-blocked task to its live executor.',
  request_format: object({ task_id: string }, ['task_id']),
  response_format: object(),
})

iii.registerFunction(
  'kanban::tasks::retry',
  async (input: { task_id: string; model?: string; profile?: string }) => {
    const task = await loadTask(input.task_id)
    if (input.model?.trim() || input.profile?.trim()) {
      await saveTask({
        ...task,
        ...(input.model?.trim() ? { model: input.model.trim() } : {}),
        ...(input.profile?.trim() ? { profile: input.profile.trim() } : {}),
        updated_at_ms: Date.now(),
      })
    }
    return dispatchTask(input.task_id, true)
  },
  {
    description: 'Retry an attention-blocked task, preserving the claimed session for managed worktrees.',
    request_format: object({ task_id: string, model: nullableString, profile: nullableString }, ['task_id']),
    response_format: object(),
  },
)

iii.registerFunction(
  'kanban::tasks::stop',
  async (input: { task_id: string }) => {
    const task = await loadTask(input.task_id)
    if (task.status !== 'running') throw new Error(`TASK_NOT_RUNNING: ${task.id}`)
    if (!task.external_session_id) throw new Error(`TASK_SESSION_MISSING: ${task.id}`)
    const executor = (await discoverExecutors()).find((candidate) => candidate.id === task.executor_id)
    if (!executor?.stop_function) throw new Error(`STOP_UNSUPPORTED: ${task.executor_id}`)
    await iii.trigger({
      function_id: executor.stop_function,
      payload: { session_id: task.external_session_id },
      timeoutMs: 15_000,
      ...(executor.namespace ? { namespace: executor.namespace } : {}),
    })
    const next: TaskRecord = {
      ...task,
      status: 'needs_you',
      error: 'Stopped by operator',
      updated_at_ms: Date.now(),
    }
    await saveTask(next)
    return next
  },
  {
    description: 'Stop a running task when its executor exposes a stop function.',
    request_format: object({ task_id: string }, ['task_id']),
    response_format: object(),
  },
)

iii.registerFunction(
  'kanban::tasks::accept',
  async (input: { task_id: string }) => {
    const task = await loadTask(input.task_id)
    if (task.status !== 'review') throw new Error(`TASK_NOT_IN_REVIEW: ${task.id}`)
    const next: TaskRecord = { ...task, status: 'done', updated_at_ms: Date.now() }
    await saveTask(next)
    return next
  },
  {
    description: 'Accept a reviewed task and move it to Done.',
    request_format: object({ task_id: string }, ['task_id']),
    response_format: object(),
  },
)

iii.registerFunction(
  'kanban::tasks::land',
  async (input: { task_id: string; target_branch?: string; test_cmd?: string; keep?: boolean }) => {
    const task = await loadTask(input.task_id)
    if (!task.worktree_id) throw new Error(`TASK_WORKTREE_MISSING: ${task.id}`)
    if (!['review', 'done'].includes(task.status)) throw new Error(`TASK_NOT_REVIEWED: ${task.id}`)
    const run = await stateGet<RunRecord>(RUNS_SCOPE, task.run_id)
    if (!validRun(run)) throw new Error(`RUN_NOT_FOUND: ${task.run_id}`)
    const targetBranch = input.target_branch?.trim() || run.target_branch
    if (!targetBranch) throw new Error('TARGET_BRANCH_REQUIRED')
    const catalog = await listFunctionRows()
    const landed = await triggerCatalogFunction<Record<string, unknown>, WorktreeLandResponse>(
      catalog,
      'worktree::land',
      {
        worktree_id: task.worktree_id,
        target_branch: targetBranch,
        ...(input.test_cmd?.trim() ? { test_cmd: input.test_cmd.trim() } : {}),
        keep: input.keep === true,
      },
      30_000,
    )
    const next: TaskRecord = {
      ...task,
      land_job_id: landed.job_id,
      land_status: 'landing',
      updated_at_ms: Date.now(),
    }
    await saveTask(next)
    return next
  },
  {
    description: 'Queue reviewed work for tested, serialized landing onto the run target branch.',
    request_format: object(
      {
        task_id: string,
        target_branch: nullableString,
        test_cmd: nullableString,
        keep: { type: 'boolean' },
      },
      ['task_id'],
    ),
    response_format: object(),
  },
)

type UiAsset = { file: string; type: 'console:script' | 'console:style'; content_type: string; content: string }

const uiAssets: Record<string, UiAsset> = {
  'kanban/page.js': { file: 'page.js', type: 'console:script', content_type: 'text/javascript', content: uiPage },
  'kanban/styles.css': { file: 'styles.css', type: 'console:style', content_type: 'text/css', content: uiStyles },
}
const uiWatch = process.env.III_KANBAN_UI_WATCH
const uiWatchEnabled = Boolean(uiWatch)
const uiWatchDir =
  uiWatchEnabled && uiWatch !== '1' && uiWatch !== 'true'
    ? (uiWatch as string)
    : join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'ui', 'dist')

async function uiContent(path: string) {
  const asset = uiAssets[path]
  if (!asset) throw new Error(`unknown ui asset: ${path}`)
  const content = uiWatchEnabled ? await readFile(join(uiWatchDir, asset.file), 'utf8') : asset.content
  return { content, content_type: asset.content_type }
}

iii.registerFunction('kanban::ui-content', (input: { path: string }) => uiContent(input.path), {
  description: 'Serve the injectable Kanban Console page assets.',
  metadata: { internal: true },
  request_format: object({ path: string }, ['path']),
  response_format: object({ content: string, content_type: string }, ['content', 'content_type']),
})

function registerUiAsset(path: string) {
  return iii.registerTrigger({ type: uiAssets[path].type, function_id: 'kanban::ui-content', config: { path } })
}

const uiTriggers = new Map(Object.keys(uiAssets).map((path) => [path, registerUiAsset(path)]))

if (uiWatchEnabled) {
  const pending = new Map<string, NodeJS.Timeout>()
  watch(uiWatchDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file)
    if (!path) return
    clearTimeout(pending.get(path))
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiTriggers.get(path)
        uiTriggers.set(path, registerUiAsset(path))
        previous?.unregister()
        console.error(`[${WORKER}] reloaded ui asset ${path}`)
      }, 150),
    )
  })
  console.error(`[${WORKER}] serving ui assets from ${uiWatchDir}`)
}

console.log(`${WORKER} worker connected`)

const shutdown = async () => {
  await iii.shutdown()
  process.exit(0)
}
process.on('SIGINT', shutdown)
process.on('SIGTERM', shutdown)
