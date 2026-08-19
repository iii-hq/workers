import type { Host } from '@iii-dev/console-ui'

export interface SessionCronTask {
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

export async function listSessionCronTasks(
  host: Host,
  sessionId: string,
): Promise<SessionCronTask[]> {
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

export async function removeSessionCronTask(
  host: Host,
  sessionId: string,
  subscriptionId: string,
): Promise<boolean> {
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
export function resolveModel(): string | undefined {
  try {
    return window.localStorage?.getItem(LAST_MODEL_KEY) ?? undefined
  } catch {
    return undefined
  }
}

export async function sendToSession(
  host: Host,
  sessionId: string,
  message: string,
  model?: string,
): Promise<void> {
  await host.iii.trigger('harness::send', {
    session_id: sessionId,
    message,
    ...(model ? { model } : {}),
  })
}

export async function listSystemCronBindings(
  host: Host,
): Promise<SystemCronBinding[]> {
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
          stringValue(config.condition_function_id)
          ?? stringValue(summaryConfig.condition_function_id),
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
