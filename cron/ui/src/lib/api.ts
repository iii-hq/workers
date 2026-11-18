import type { Host } from '@iii-dev/console-ui'

/** The function every schedule observes; its args are `{ expression }`. */
export const ON_TICK_FN = 'cron::on-tick'

/** The harness's own observer target: observers that wake a session. */
const HARNESS_DELIVER_FN = 'harness::observer::deliver'

export interface SessionCronTask {
  /** The session a fire wakes. Every schedule the page creates gets its own,
      so a routine never lands in whatever chat happened to be open. */
  sessionId: string
  sessionTitle?: string
  /** Made by an agent in an ordinary chat (not by this page, which gives each
      schedule a session of its own): the row names that chat. */
  fromChat?: boolean
  observerId: string
  expression: string
  target?: string
  label?: string
  conditions: unknown[]
  once: boolean
  maxFires?: number
  expiresAt?: number
  fires: number
  createdAt: number
}

/** An observer of `cron::on-tick` that a worker created for one of its own
    functions, as `engine::observers::list` reports it. */
export interface SystemCronObserver {
  id: string
  /** The function each fire calls. */
  functionId: string
  workerName: string
  expression: string
  state: string
  fires: number
  filters?: Record<string, unknown>
}

export interface FunctionSummary {
  functionId: string
  workerName: string
  description?: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

function numberValue(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function rows(value: unknown, key: string): unknown[] {
  return isRecord(value) && Array.isArray(value[key]) ? value[key] : []
}

/** A harness row's target: a function id, or `{ function_id }`. */
function targetFunctionId(value: unknown): string | undefined {
  return isRecord(value) ? stringValue(value.function_id) : stringValue(value)
}

async function listSessionCronTasks(host: Host, sessionId: string): Promise<SessionCronTask[]> {
  const response = await host.iii.call('harness::observers::list', {
    session_id: sessionId,
  })
  return rows(response, 'observers')
    .map((row): SessionCronTask | null => {
      if (!isRecord(row) || row.function_id !== ON_TICK_FN) return null
      const observerId = stringValue(row.observer_id)
      const args = isRecord(row.args) ? row.args : {}
      const expression = stringValue(args.expression)
      if (!observerId || !expression) return null
      return {
        sessionId,
        observerId,
        expression,
        target: targetFunctionId(row.target),
        label: stringValue(row.label),
        conditions: Array.isArray(row.conditions) ? row.conditions : [],
        once: row.once === true,
        maxFires: numberValue(row.max_fires),
        expiresAt: numberValue(row.expires_at),
        fires: numberValue(row.fires) ?? 0,
        createdAt: numberValue(row.created_at) ?? 0,
      }
    })
    .filter((task): task is SessionCronTask => task !== null)
    .sort((left, right) => right.createdAt - left.createdAt)
}

export async function removeSessionCronTask(host: Host, sessionId: string, observerId: string): Promise<boolean> {
  const response = await host.iii.call('harness::observers::dispose', {
    session_id: sessionId,
    observer_id: observerId,
  })
  return isRecord(response) && response.removed === true
}

const LAST_MODEL_KEY = 'iii-chat-last-model'

/** A session that has never taken a turn has no model to inherit, so the send
    has to name one, and the console's own last pick is the only defensible
    source. Picking off the catalogue instead looks helpful and is not: the
    first entry is arbitrary, and a model that cannot emit function calls
    burns the whole turn failing to register anything. Undefined here means
    the caller asks the operator to choose. */
export function resolveModel(host: Host, conversationId?: string | null): string | undefined {
  // The console knows what the operator picked, including a draft they have
  // not sent yet. Storage is the fallback for a console that predates the
  // chat host API.
  const live = host.chat?.composerModel?.(conversationId ?? undefined)
  if (live) return live
  try {
    return window.localStorage?.getItem(LAST_MODEL_KEY) ?? undefined
  } catch {
    return undefined
  }
}

export async function sendToSession(host: Host, sessionId: string, message: string, model?: string): Promise<void> {
  await host.iii.call('harness::send', {
    session_id: sessionId,
    message,
    ...(model ? { model } : {}),
  })
}

/** Marks a session as one this page created to own a schedule, so the list
    can find every routine again without depending on the open conversation. */
export const SCHEDULE_SESSION_METADATA = { cron_ui: true, kind: 'schedule' }

interface ScheduleSession {
  sessionId: string
  title?: string
}

const SESSION_PAGE = 200

/** Reading every session at once would open one socket per schedule. */
const READ_CONCURRENCY = 8

/** Every session this page created, followed across pages. The metadata
    filter is applied by session-manager, so an operator with hundreds of
    chats does not lose schedules behind a page of unrelated conversations. */
async function listScheduleSessions(host: Host): Promise<ScheduleSession[]> {
  const found: ScheduleSession[] = []
  const seenCursors = new Set<string>()
  let cursor: string | undefined
  for (;;) {
    const response = await host.iii.call('session::list', {
      metadata: { cron_ui: true },
      limit: SESSION_PAGE,
      ...(cursor ? { cursor } : {}),
    })
    for (const row of rows(response, 'sessions')) {
      if (!isRecord(row)) continue
      const sessionId = stringValue(row.session_id)
      if (sessionId) found.push({ sessionId, title: stringValue(row.title) })
    }
    const next = isRecord(response) ? stringValue(response.next_cursor) : undefined
    // A repeated cursor means the store is paging in circles; stop rather
    // than fetch forever.
    if (!next || seenCursors.has(next)) return found
    seenCursors.add(next)
    cursor = next
  }
}

/** Read many sessions' schedules, `READ_CONCURRENCY` at a time. One
    unreadable session must not blank the whole list. */
async function readSchedules(
  host: Host,
  sessions: ReadonlyMap<string, string | undefined>,
): Promise<SessionCronTask[]> {
  const pending = [...sessions]
  const collected: SessionCronTask[] = []
  const readers = Array.from({ length: Math.min(READ_CONCURRENCY, pending.length) }, async () => {
    for (;;) {
      const entry = pending.pop()
      if (!entry) return
      const [sessionId, title] = entry
      try {
        for (const task of await listSessionCronTasks(host, sessionId)) {
          collected.push({ ...task, sessionTitle: title })
        }
      } catch {
        // One unreadable session must not blank the whole list.
      }
    }
  })
  await Promise.all(readers)
  return collected
}

/** The ids of every harness-owned observer of `cron::on-tick`: every
    schedule an agent made with `harness::observe`, in whatever chat. The
    engine knows these exist but not who owns them (their target is the
    harness's delivery hop), so the owner is found by the session scan below. */
async function harnessScheduleIds(host: Host): Promise<Set<string>> {
  const response = await host.iii.call('engine::observers::list', {
    function_id: ON_TICK_FN,
  })
  const ids = new Set<string>()
  for (const row of rows(response, 'observers')) {
    if (!isRecord(row) || row.function_id !== ON_TICK_FN) continue
    const id = stringValue(row.id)
    if (id && targetFunctionId(row.target) === HARNESS_DELIVER_FN) ids.add(id)
  }
  return ids
}

/** Observer id → owning chat, remembered across refreshes so a schedule made
    in an old chat is found again without re-scanning every session. */
const knownOwners = new Map<string, { sessionId: string; title?: string }>()

/** Find the chats that own `missing` schedules: most recently updated first
    (an agent's schedule lives in a chat someone just used), stopping as soon
    as every one is found. */
async function findScheduleOwners(
  host: Host,
  missing: Set<string>,
  alreadyRead: Set<string>,
): Promise<SessionCronTask[]> {
  const found: SessionCronTask[] = []
  const take = (tasks: SessionCronTask[]) => {
    for (const task of tasks) {
      knownOwners.set(task.observerId, { sessionId: task.sessionId, title: task.sessionTitle })
      if (missing.delete(task.observerId)) found.push(task)
    }
  }
  // Remembered owners first: one read each.
  const remembered = new Map<string, string | undefined>()
  for (const id of missing) {
    const owner = knownOwners.get(id)
    if (owner && !alreadyRead.has(owner.sessionId)) remembered.set(owner.sessionId, owner.title)
  }
  for (const sessionId of remembered.keys()) alreadyRead.add(sessionId)
  take(await readSchedules(host, remembered))

  const seenCursors = new Set<string>()
  let cursor: string | undefined
  while (missing.size > 0) {
    const response = await host.iii.call('session::list', {
      limit: SESSION_PAGE,
      ...(cursor ? { cursor } : {}),
    })
    const page = new Map<string, string | undefined>()
    for (const row of rows(response, 'sessions')) {
      if (!isRecord(row)) continue
      const sessionId = stringValue(row.session_id)
      if (!sessionId || alreadyRead.has(sessionId)) continue
      alreadyRead.add(sessionId)
      page.set(sessionId, stringValue(row.title))
    }
    take(await readSchedules(host, page))
    const next = isRecord(response) ? stringValue(response.next_cursor) : undefined
    if (!next || seenCursors.has(next)) break
    seenCursors.add(next)
    cursor = next
  }
  return found
}

/** Every schedule: the ones this page made (each in a session of its own),
    the open conversation's, and every one an agent made in any other chat —
    a person looks for a job they asked a chat to schedule here, on the Cron
    page. The last are found only when the engine reports a harness schedule
    the first two reads did not, so an operator with no such schedule pays
    for one extra call. */
export async function listAllSchedules(host: Host, conversationId?: string): Promise<SessionCronTask[]> {
  const sessions = await listScheduleSessions(host)
  const ids = new Map<string, string | undefined>()
  for (const session of sessions) ids.set(session.sessionId, session.title)
  if (conversationId && !ids.has(conversationId)) {
    ids.set(conversationId, undefined)
  }
  const pageSessions = new Set(sessions.map((session) => session.sessionId))
  const collected = (await readSchedules(host, ids)).map((task) =>
    pageSessions.has(task.sessionId) ? task : { ...task, fromChat: true },
  )

  let harnessIds: Set<string>
  try {
    harnessIds = await harnessScheduleIds(host)
  } catch {
    // Without the engine's list the page still shows its own schedules.
    return collected
  }
  for (const task of collected) harnessIds.delete(task.observerId)
  if (harnessIds.size === 0) return collected
  try {
    const found = await findScheduleOwners(host, harnessIds, new Set(ids.keys()))
    return [...collected, ...found.map((task) => ({ ...task, fromChat: true }))]
  } catch {
    return collected
  }
}

/** The agent's observe surface: `harness::observe` creates the schedule's
    observer, `harness::unobserve` retires the one it replaces. */
const SCHEDULE_CALLS = ['harness::observe', 'harness::unobserve']

/** What a schedule session's policy must allow. The harness refuses to
    observe a function the session's policy does not allow ("an observer
    reads every call of the function it observes"), and a call delivery's
    target must be callable by the observer too — without both, the agent's
    one `harness::observe` is refused and nothing is scheduled. */
export function scheduleSessionAllow(target?: string): string[] {
  return [...SCHEDULE_CALLS, ON_TICK_FN, ...(target ? [target] : [])]
}

/** Create a schedule in a session of its own. The policy is exactly the two
    calls a schedule needs, and the gate is set to accept them, because the
    operator asked for this by pressing the button. */
export async function createSchedule(
  host: Host,
  input: { title: string; instruction: string; model: string; target?: string },
): Promise<string> {
  const response = await host.iii.call('harness::send', {
    message: input.instruction,
    model: input.model,
    session: {
      title: input.title,
      metadata: SCHEDULE_SESSION_METADATA,
    },
    options: {
      max_turns: 6,
      functions: {
        allow: scheduleSessionAllow(input.target),
        expose: 'native',
      },
    },
  })
  const sessionId = isRecord(response) ? stringValue(response.session_id) : undefined
  if (!sessionId) {
    throw new Error('Harness accepted the request without naming a session')
  }
  return sessionId
}

/** The approval gate's functions, when a worker provides them. */
const APPROVE_ALWAYS_FN = 'approval::approve-always'
const LIST_PENDING_FN = 'approval::list-pending'
const RESOLVE_FN = 'approval::resolve'

/** Whether every one of `functionIds` is registered. A call to a missing
    function is logged by the engine as an error, and Sentinel turns each into
    an issue: a new user's stack without the approval gate was filling up with
    "approval::approve-always: Function not found". */
export async function functionsRegistered(host: Host, functionIds: readonly string[]): Promise<boolean> {
  try {
    const response = await host.iii.call('engine::functions::list', {
      include_internal: true,
    })
    const present = new Set(
      rows(response, 'functions')
        .map((row) => (isRecord(row) ? stringValue(row.function_id) : undefined))
        .filter((id): id is string => id !== undefined),
    )
    return functionIds.every((id) => present.has(id))
  } catch {
    return false
  }
}

/** Grant exactly the two calls a schedule makes, rather than putting the
    session in full-permission mode: the schedule session stays a real chat the
    operator can type into, and a blanket grant would outlive this turn.

    Sequential, not concurrent: both grants edit one settings record, and
    issued together the second read starts before the first write lands, so one
    grant disappears.

    A turn that reached the gate before the grants did leaves a held call, so
    anything already waiting for these two functions is released here — the
    operator asked for this schedule by pressing the button.

    Only when the approval gate is deployed: with none there is nothing to
    grant (no call is held), and calling it anyway logs an engine error. */
export async function allowScheduleCalls(host: Host, sessionId: string): Promise<void> {
  if (!(await functionsRegistered(host, [APPROVE_ALWAYS_FN, LIST_PENDING_FN, RESOLVE_FN]))) return
  for (const functionId of SCHEDULE_CALLS) {
    try {
      await host.iii.call(APPROVE_ALWAYS_FN, {
        session_id: sessionId,
        function_id: functionId,
      })
    } catch {
      return
    }
  }
  await releaseHeldScheduleCalls(host, sessionId)
}

async function releaseHeldScheduleCalls(host: Host, sessionId: string): Promise<void> {
  try {
    const response = await host.iii.call(LIST_PENDING_FN, {
      session_id: sessionId,
    })
    for (const row of rows(response, 'pending')) {
      if (!isRecord(row)) continue
      const functionId = stringValue(row.function_id)
      const callId = stringValue(row.function_call_id)
      if (!callId || !functionId || !SCHEDULE_CALLS.includes(functionId)) continue
      await host.iii.call(RESOLVE_FN, {
        session_id: sessionId,
        function_call_id: callId,
        decision: 'allow',
      })
    }
  } catch {
    // Nothing held: the turn proceeds either way.
  }
}

export type ScheduleOutcome =
  | { status: 'created'; task: SessionCronTask }
  | { status: 'failed'; reply?: string }
  | { status: 'pending' }

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms))

/** The agent's last words in a session, to say why a schedule was not made. */
async function lastReply(host: Host, sessionId: string): Promise<string | undefined> {
  try {
    const response = await host.iii.call('session::messages', { session_id: sessionId })
    const entries = rows(response, 'messages')
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const entry = entries[index]
      const message = isRecord(entry) && isRecord(entry.message) ? entry.message : undefined
      if (message?.role !== 'assistant' || !Array.isArray(message.content)) continue
      const text = message.content
        .map((block) => (isRecord(block) && block.type === 'text' ? stringValue(block.text) : undefined))
        .filter(Boolean)
        .join('\n')
        .trim()
      if (text) return text
    }
  } catch {
    // The reply is a courtesy; the outcome stands without it.
  }
  return undefined
}

async function sessionStatus(host: Host, sessionId: string): Promise<string | undefined> {
  try {
    const response = await host.iii.call('session::get', { session_id: sessionId })
    const meta = isRecord(response) && isRecord(response.meta) ? response.meta : response
    return isRecord(meta) ? stringValue(meta.status) : undefined
  } catch {
    return undefined
  }
}

/** Follow a schedule session's turn to its outcome. `harness::send` returns
    as soon as the turn is seeded, so "sent" is not "scheduled": the schedule
    exists once its session owns a `cron::on-tick` observer, and has failed
    once the turn ended without one (the agent's reply says why). Still
    neither after `timeoutMs` is `pending`. */
export async function waitForSchedule(
  host: Host,
  sessionId: string,
  {
    timeoutMs = 120_000,
    intervalMs = 1_500,
    sleep = delay,
    cancelled = () => false,
  }: {
    timeoutMs?: number
    intervalMs?: number
    sleep?: (ms: number) => Promise<void>
    cancelled?: () => boolean
  } = {},
): Promise<ScheduleOutcome> {
  const newest = (tasks: SessionCronTask[]) =>
    tasks.reduce<SessionCronTask | undefined>((best, task) => (!best || task.createdAt > best.createdAt ? task : best), undefined)
  for (let waited = 0; ; waited += intervalMs) {
    if (cancelled()) return { status: 'pending' }
    const task = newest(await listSessionCronTasks(host, sessionId).catch(() => []))
    if (task) return { status: 'created', task }
    const status = await sessionStatus(host, sessionId)
    if (status === 'done' || status === 'error') {
      // The observer may have landed between the two reads.
      const late = newest(await listSessionCronTasks(host, sessionId).catch(() => []))
      if (late) return { status: 'created', task: late }
      return { status: 'failed', reply: await lastReply(host, sessionId) }
    }
    if (waited >= timeoutMs) return { status: 'pending' }
    await sleep(intervalMs)
  }
}

/** Every observer of `cron::on-tick` that a worker made for its own
    functions. The harness's observers are the schedules above. */
export async function listSystemCronObservers(host: Host): Promise<SystemCronObserver[]> {
  const response = await host.iii.call('engine::observers::list', {
    function_id: ON_TICK_FN,
  })
  return rows(response, 'observers')
    .map((row): SystemCronObserver | null => {
      if (!isRecord(row) || row.function_id !== ON_TICK_FN) return null
      const id = stringValue(row.id)
      const functionId = targetFunctionId(row.target)
      if (!id || !functionId || functionId === HARNESS_DELIVER_FN) return null
      const args = isRecord(row.args) ? row.args : {}
      const expression = stringValue(args.expression)
      if (!expression) return null
      const worker = isRecord(row.worker) ? row.worker : {}
      const filters = isRecord(row.filters) && Object.keys(row.filters).length > 0 ? row.filters : undefined
      return {
        id,
        functionId,
        workerName: stringValue(worker.worker_name) ?? stringValue(worker.worker_id) ?? 'unknown',
        expression,
        state: stringValue(row.state) ?? 'unknown',
        fires: numberValue(row.fires) ?? 0,
        filters,
      }
    })
    .filter((observer): observer is SystemCronObserver => observer !== null)
    .sort((left, right) => left.functionId.localeCompare(right.functionId))
}

export async function listFunctions(host: Host): Promise<FunctionSummary[]> {
  const response = await host.iii.call('engine::functions::list', {
    include_internal: false,
  })
  return rows(response, 'functions')
    .map((row): FunctionSummary | null => {
      if (!isRecord(row)) return null
      const functionId = stringValue(row.function_id)
      // A private function cannot be an observer's target.
      if (!functionId || functionId.startsWith('harness::') || row.private === true) return null
      return {
        functionId,
        workerName: stringValue(row.worker_name) ?? 'unknown',
        description: stringValue(row.description),
      }
    })
    .filter((fn): fn is FunctionSummary => fn !== null)
    .sort((left, right) => left.functionId.localeCompare(right.functionId))
}
