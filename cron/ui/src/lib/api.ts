import type { Host } from '@iii-dev/console-ui'

export interface SessionCronTask {
  /** The session a fire wakes. Every schedule the page creates gets its own,
      so a routine never lands in whatever chat happened to be open. */
  sessionId: string
  sessionTitle?: string
  subscriptionId: string
  triggerId?: string
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

export interface SystemCronBinding {
  id: string
  functionId: string
  workerName: string
  expression: string
  conditionFunctionId?: string
  configSummary?: string
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

function jsonRecord(value: string | undefined): Record<string, unknown> {
  if (!value) return {}
  try {
    const parsed: unknown = JSON.parse(value)
    return isRecord(parsed) ? parsed : {}
  } catch {
    return {}
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  if (isRecord(error)) {
    for (const key of ['message', 'error', 'reason', 'detail']) {
      const candidate = error[key]
      if (typeof candidate === 'string' && candidate.trim()) return candidate
    }
  }
  try {
    return JSON.stringify(error) ?? String(error)
  } catch {
    return String(error)
  }
}

async function listSessionCronTasks(host: Host, sessionId: string): Promise<SessionCronTask[]> {
  const response = await host.iii.trigger('harness::triggers::list', {
    session_id: sessionId,
  })
  return rows(response, 'subscriptions')
    .map((row): SessionCronTask | null => {
      if (!isRecord(row) || row.trigger_type !== 'cron') return null
      const subscriptionId = stringValue(row.subscription_id)
      const config = isRecord(row.config) ? row.config : {}
      const expression = stringValue(config.expression)
      if (!subscriptionId || !expression) return null
      return {
        sessionId,
        subscriptionId,
        triggerId: stringValue(row.trigger_id),
        expression,
        target: stringValue(row.target),
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

export async function removeSessionCronTask(host: Host, sessionId: string, subscriptionId: string): Promise<boolean> {
  const response = await host.iii.trigger('harness::triggers::unregister', {
    session_id: sessionId,
    subscription_id: subscriptionId,
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
  await host.iii.trigger('harness::send', {
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
    const response = await host.iii.trigger('session::list', {
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

/** Create a schedule in a session of its own. The policy is exactly the two
    calls a registration needs, and the gate is set to accept them, because the
    operator asked for this by pressing the button. */
export async function createSchedule(
  host: Host,
  input: { title: string; instruction: string; model: string },
): Promise<string> {
  const response = await host.iii.trigger('harness::send', {
    message: input.instruction,
    model: input.model,
    session: {
      title: input.title,
      metadata: SCHEDULE_SESSION_METADATA,
    },
    options: {
      mode: 'agent',
      max_turns: 6,
      functions: {
        allow: ['engine::register_trigger', 'engine::unregister_trigger'],
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

const SCHEDULE_CALLS = ['engine::register_trigger', 'engine::unregister_trigger']

/** Grant exactly the two calls a registration makes, rather than putting the
    session in full-permission mode: the schedule session stays a real chat the
    operator can type into, and a blanket grant would outlive this turn.

    Sequential, not concurrent: both grants edit one settings record, and
    issued together the second read starts before the first write lands, so one
    grant disappears.

    A turn that reached the gate before the grants did leaves a held call, so
    anything already waiting for these two functions is released here — the
    operator asked for this schedule by pressing the button.

    Best effort throughout: with no approval gate deployed there is nothing to
    grant, and the registration simply waits for a human instead. */
export async function allowScheduleCalls(host: Host, sessionId: string): Promise<void> {
  for (const functionId of SCHEDULE_CALLS) {
    try {
      await host.iii.trigger('approval::approve-always', {
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
    const response = await host.iii.trigger('approval::list-pending', {
      session_id: sessionId,
    })
    for (const row of rows(response, 'pending')) {
      if (!isRecord(row)) continue
      const functionId = stringValue(row.function_id)
      const callId = stringValue(row.function_call_id)
      if (!callId || !functionId || !SCHEDULE_CALLS.includes(functionId)) continue
      await host.iii.trigger('approval::resolve', {
        session_id: sessionId,
        function_call_id: callId,
        decision: 'allow',
      })
    }
  } catch {
    // No gate, or nothing held: the turn proceeds either way.
  }
}

export async function listSystemCronBindings(host: Host): Promise<SystemCronBinding[]> {
  const response = await host.iii.trigger('engine::registered-triggers::list', {
    include_internal: false,
    trigger_type: 'cron',
  })
  return rows(response, 'registered_triggers')
    .map((row): SystemCronBinding | null => {
      if (!isRecord(row) || row.trigger_type !== 'cron') return null
      const id = stringValue(row.id)
      const functionId = stringValue(row.function_id)
      if (!id || !functionId || functionId === 'harness::trigger::deliver') {
        return null
      }
      const configSummary = stringValue(row.config_summary)
      const summaryConfig = jsonRecord(configSummary)
      const config = isRecord(row.config) ? row.config : {}
      const expression = stringValue(config.expression) ?? stringValue(summaryConfig.expression)
      if (!expression) return null
      return {
        id,
        functionId,
        workerName: stringValue(row.worker_name) ?? 'unknown',
        expression,
        conditionFunctionId:
          stringValue(config.condition_function_id) ?? stringValue(summaryConfig.condition_function_id),
        configSummary,
      }
    })
    .filter((binding): binding is SystemCronBinding => binding !== null)
    .sort((left, right) => left.functionId.localeCompare(right.functionId))
}

export async function listFunctions(host: Host): Promise<FunctionSummary[]> {
  const response = await host.iii.trigger('engine::functions::list', {
    include_internal: false,
  })
  return rows(response, 'functions')
    .map((row): FunctionSummary | null => {
      if (!isRecord(row)) return null
      const functionId = stringValue(row.function_id)
      if (!functionId || functionId.startsWith('harness::')) return null
      return {
        functionId,
        workerName: stringValue(row.worker_name) ?? 'unknown',
        description: stringValue(row.description),
      }
    })
    .filter((fn): fn is FunctionSummary => fn !== null)
    .sort((left, right) => left.functionId.localeCompare(right.functionId))
}
