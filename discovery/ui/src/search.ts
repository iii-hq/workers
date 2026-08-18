import type { JsonValue } from '@iii-dev/console-ui'

/** Parsed `discovery::search_functions` result as the call card renders it. */
export interface DiscoverContractView {
  function_id: string
  description: string
  request_schema: JsonValue
}

export interface DiscoverWorkerView {
  namespace: string
  functions: DiscoverContractView[]
}

export interface DiscoverView {
  guidance: string
  workers: DiscoverWorkerView[]
  latency_ms: number
}

/** `{ content: [...], details }` harness result envelope → details.
 * Idempotent: an already-flat payload passes through unchanged. Port of the
 * console's helper — injected bundles copy, not import. */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return value
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) return obj.details
  return value
}

/** Error-shaped output (engine `{ error: ... }` wrapper). The renderer
 * returns null for these so the console's default error cards apply. */
export function isErrorOutput(value: unknown): boolean {
  return (
    !!value &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    'error' in (value as Record<string, unknown>)
  )
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

/** The request's `query`, tolerating agents that double-encode the payload
 * as a JSON string. Null when no readable query exists. */
export function discoverQuery(input: unknown): string | null {
  let value = input
  if (typeof value === 'string') {
    try {
      value = JSON.parse(value)
    } catch {
      return null
    }
  }
  if (!isRecord(value) || typeof value.query !== 'string') return null
  const query = value.query.trim()
  return query.length > 0 ? query : null
}

/** Strict parse of a settled discover output (envelope tolerated). Null on
 * any shape drift so the card falls back to the generic JSON panes. */
export function parseDiscoverResponse(output: unknown): DiscoverView | null {
  const value = unwrapEnvelope(output)
  if (!isRecord(value)) return null
  if (typeof value.guidance !== 'string') return null
  if (typeof value.latency_ms !== 'number' || !Number.isFinite(value.latency_ms)) return null
  if (!Array.isArray(value.workers)) return null
  const workers: DiscoverWorkerView[] = []
  for (const worker of value.workers) {
    if (!isRecord(worker)) return null
    if (typeof worker.namespace !== 'string' || worker.namespace.length === 0) return null
    if (!Array.isArray(worker.functions)) return null
    const functions: DiscoverContractView[] = []
    for (const contract of worker.functions) {
      if (!isRecord(contract)) return null
      if (typeof contract.function_id !== 'string' || contract.function_id.length === 0) {
        return null
      }
      if (typeof contract.description !== 'string') return null
      if (!('request_schema' in contract)) return null
      functions.push({
        function_id: contract.function_id,
        description: contract.description,
        request_schema: contract.request_schema as JsonValue,
      })
    }
    workers.push({ namespace: worker.namespace, functions })
  }
  return { guidance: value.guidance, workers, latency_ms: value.latency_ms }
}

/** Schemas that constrain nothing (missing/empty/bare `type: object`) —
 * rendered as `· any` instead of an expandable body. */
export function schemaIsAny(schema: JsonValue): boolean {
  if (!isRecord(schema)) return true
  const keys = Object.keys(schema).filter(
    (key) => key !== '$schema' && key !== 'title' && key !== 'type',
  )
  if (keys.length > 0) return false
  return schema.type === undefined || schema.type === 'object'
}

export function functionCount(view: DiscoverView): number {
  return view.workers.reduce((total, worker) => total + worker.functions.length, 0)
}
