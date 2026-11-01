/**
 * OTel logs RPC adapter.
 *
 * Engine contract: `engine::logs::list` is exposed by the engine's
 * observability worker (see `motia/engine/src/workers/observability/mod.rs`).
 * Same response shape as motia's HTTP `/otel/logs` endpoint, called through
 * the iii-browser-sdk WebSocket so the chat app's single transport handles
 * both traces and logs.
 *
 * When the engine has no log storage initialized (logs disabled), the
 * function returns an empty `{ logs: [], total: 0 }` payload — no error
 * thrown. The UI degrades to an empty state without a special path.
 */

import { getIiiClient } from '@/lib/iii-client'

export interface OtelLog {
  timestamp_unix_nano: number
  observed_timestamp_unix_nano: number
  trace_id?: string | null
  span_id?: string | null
  severity_number: number
  severity_text: string
  body: string
  attributes: Record<string, unknown>
  resource: Record<string, unknown>
  instrumentation_scope_name?: string
  instrumentation_scope_version?: string
  service_name?: string
}

export interface OtelLogsResponse {
  logs: OtelLog[]
  total: number
  query?: {
    start_time?: number
    end_time?: number
    trace_id?: string
    span_id?: string
    severity_min?: number
    severity_text?: string
    offset: number
    limit: number
  }
  timestamp: number
}

export interface OtelLogsFilterParams {
  start_time?: number
  end_time?: number
  trace_id?: string
  span_id?: string
  severity_min?: number
  severity_text?: string
  offset?: number
  limit?: number
}

const FN_LOGS_LIST = 'engine::logs::list'
const FN_LOGS_CLEAR = 'engine::logs::clear'

function stripUndefined<T extends Record<string, unknown>>(obj: T): T {
  return Object.fromEntries(
    Object.entries(obj).filter(([, v]) => v !== undefined),
  ) as T
}

function asError(err: unknown, fallback: string): Error {
  if (err instanceof Error) return err
  return new Error(typeof err === 'string' ? err : fallback)
}

function isLogsUnavailable(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err)
  return /function not (registered|found)|logs (are )?not enabled/i.test(msg)
}

export async function fetchOtelLogs(
  options?: OtelLogsFilterParams,
): Promise<OtelLogsResponse> {
  const offset = options?.offset ?? 0
  const limit = options?.limit ?? 100
  const payload = stripUndefined({ ...options, offset, limit })

  try {
    const client = await getIiiClient()
    return await client.call<OtelLogsResponse>(FN_LOGS_LIST, payload)
  } catch (err) {
    if (isLogsUnavailable(err)) {
      return { logs: [], total: 0, timestamp: Date.now() }
    }
    throw asError(err, 'Failed to fetch OTEL logs')
  }
}

export async function clearOtelLogs(): Promise<{ success: boolean }> {
  try {
    const client = await getIiiClient()
    await client.call(FN_LOGS_CLEAR, {})
    return { success: true }
  } catch (err) {
    throw asError(err, 'Failed to clear OTEL logs')
  }
}
