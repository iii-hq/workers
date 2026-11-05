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

/** Every schedule this page owns, plus the open conversation's own, so a
    routine created from chat is not hidden here. */
export async function listAllSchedules(host: Host, conversationId?: string): Promise<SessionCronTask[]> {
  const sessions = await listScheduleSessions(host)
  const ids = new Map<string, string | undefined>()
  for (const session of sessions) ids.set(session.sessionId, session.title)
  if (conversationId && !ids.has(conversationId)) {
    ids.set(conversationId, undefined)
  }

  const pending = [...ids]
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

/** The agent's observe surface: `harness::observe` creates the schedule's
    observer, `harness::unobserve` retires the one it replaces. */
const SCHEDULE_CALLS = ['harness::observe', 'harness::unobserve']

/** Create a schedule in a session of its own. The policy is exactly the two
    calls a schedule needs, and the gate is set to accept them, because the
    operator asked for this by pressing the button. */
export async function createSchedule(
  host: Host,
  input: { title: string; instruction: string; model: string },
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
        allow: SCHEDULE_CALLS,
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

/** Grant exactly the two calls a schedule makes, rather than putting the
    session in full-permission mode: the schedule session stays a real chat the
    operator can type into, and a blanket grant would outlive this turn.

    Sequential, not concurrent: both grants edit one settings record, and
    issued together the second read starts before the first write lands, so one
    grant disappears.

    A turn that reached the gate before the grants did leaves a held call, so
    anything already waiting for these two functions is released here — the
    operator asked for this schedule by pressing the button.

    Best effort throughout: with no approval gate deployed there is nothing to
    grant, and the call simply waits for a human instead. */
export async function allowScheduleCalls(host: Host, sessionId: string): Promise<void> {
  for (const functionId of SCHEDULE_CALLS) {
    try {
      await host.iii.call('approval::approve-always', {
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
    const response = await host.iii.call('approval::list-pending', {
      session_id: sessionId,
    })
    for (const row of rows(response, 'pending')) {
      if (!isRecord(row)) continue
      const functionId = stringValue(row.function_id)
      const callId = stringValue(row.function_call_id)
      if (!callId || !functionId || !SCHEDULE_CALLS.includes(functionId)) continue
      await host.iii.call('approval::resolve', {
        session_id: sessionId,
        function_call_id: callId,
        decision: 'allow',
      })
    }
  } catch {
    // No gate, or nothing held: the turn proceeds either way.
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
