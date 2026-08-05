/**
 * The engine catalogue calls behind the Functions and Triggers pages, plus
 * the narrow runtime guards that keep an unexpected wire shape from
 * reaching React as `undefined.map`.
 *
 * Wire source: `iii/engine/src/workers/engine_fn/mod.rs`. Everything here is
 * read-only except `invoke`, which is a plain function call over the tab's
 * bus (`host.iii.trigger`) — the same privilege any worker on the bus has.
 *
 * Parsing is deliberately permissive: unknown fields pass through, absent
 * optionals stay absent, and only the fields the pages actually render are
 * required. A row the engine grew a field for still lists.
 */

import type { Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

export interface FunctionSummary {
  function_id: string
  worker_name: string
  description?: string | null
}

/** Inline trigger payload on `FunctionDetail` — raw `config`, not a summary. */
export interface RegisteredTriggerRef {
  id: string
  trigger_type: string
  config?: unknown
}

export interface FunctionDetail extends FunctionSummary {
  request_schema?: unknown
  response_schema?: unknown
  metadata?: unknown
  registered_triggers: RegisteredTriggerRef[]
}

/** A trigger TYPE (the catalogue entry a worker publishes). */
export interface TriggerTypeSummary {
  id: string
  worker_name: string
  description?: string | null
}

export interface TriggerTypeDetail extends TriggerTypeSummary {
  /** Live bindings of this type. */
  instance_count?: number
  /** Per-binding `config` shape accepted by `engine::register_trigger`. */
  configuration_schema?: unknown
  /** Payload shape delivered to the bound function when the trigger fires. */
  request_schema?: unknown
}

/** A live binding of a trigger type to a function. */
export interface RegisteredTrigger {
  id: string
  trigger_type: string
  function_id: string
  worker_name: string
  /** The engine sends both the raw object and a stringified summary. */
  config?: unknown
  config_summary?: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

/** `null` and absent both mean "no description"; anything else is dropped. */
function description(value: unknown): string | null | undefined {
  if (value === null) return null
  return str(value)
}

function rows(value: unknown, key: string): unknown[] {
  if (!isRecord(value)) return []
  const list = value[key]
  return Array.isArray(list) ? list : []
}

function functionSummary(row: unknown): FunctionSummary | null {
  if (!isRecord(row)) return null
  const function_id = str(row.function_id)
  if (!function_id) return null
  return {
    function_id,
    worker_name: str(row.worker_name) ?? 'unknown',
    description: description(row.description),
  }
}

function triggerRef(row: unknown): RegisteredTriggerRef | null {
  if (!isRecord(row)) return null
  const id = str(row.id)
  const trigger_type = str(row.trigger_type)
  if (!id || !trigger_type) return null
  return { id, trigger_type, config: row.config }
}

export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

export async function listFunctions(
  host: Host,
  options: { includeInternal: boolean },
): Promise<FunctionSummary[]> {
  const out = await host.iii.trigger('engine::functions::list', {
    include_internal: options.includeInternal,
  })
  return rows(out, 'functions')
    .map(functionSummary)
    .filter((f): f is FunctionSummary => f !== null)
}

export async function functionInfo(
  host: Host,
  functionId: string,
): Promise<FunctionDetail> {
  const out = await host.iii.trigger('engine::functions::info', {
    function_id: functionId,
  })
  const base = functionSummary(out)
  if (!base) throw new Error(`engine::functions::info returned no detail`)
  const detail = out as Record<string, unknown>
  return {
    ...base,
    request_schema: detail.request_schema,
    response_schema: detail.response_schema,
    metadata: detail.metadata,
    registered_triggers: rows(out, 'registered_triggers')
      .map(triggerRef)
      .filter((t): t is RegisteredTriggerRef => t !== null),
  }
}

export async function listTriggerTypes(
  host: Host,
  options: { includeInternal: boolean },
): Promise<TriggerTypeSummary[]> {
  const out = await host.iii.trigger('engine::triggers::list', {
    include_internal: options.includeInternal,
  })
  return rows(out, 'triggers')
    .map((row): TriggerTypeSummary | null => {
      if (!isRecord(row)) return null
      const id = str(row.id)
      if (!id) return null
      return {
        id,
        worker_name: str(row.worker_name) ?? 'unknown',
        description: description(row.description),
      }
    })
    .filter((t): t is TriggerTypeSummary => t !== null)
}

export async function triggerTypeInfo(
  host: Host,
  id: string,
): Promise<TriggerTypeDetail> {
  const out = await host.iii.trigger('engine::triggers::info', { id })
  if (!isRecord(out))
    throw new Error('engine::triggers::info returned no detail')
  return {
    id: str(out.id) ?? id,
    worker_name: str(out.worker_name) ?? 'unknown',
    description: description(out.description),
    instance_count:
      typeof out.instance_count === 'number' ? out.instance_count : undefined,
    configuration_schema: out.configuration_schema,
    request_schema: out.request_schema,
  }
}

export async function listRegisteredTriggers(
  host: Host,
  options: { includeInternal: boolean },
): Promise<RegisteredTrigger[]> {
  const out = await host.iii.trigger('engine::registered-triggers::list', {
    include_internal: options.includeInternal,
  })
  return rows(out, 'registered_triggers')
    .map((row): RegisteredTrigger | null => {
      if (!isRecord(row)) return null
      const id = str(row.id)
      const trigger_type = str(row.trigger_type)
      if (!id || !trigger_type) return null
      return {
        id,
        trigger_type,
        function_id: str(row.function_id) ?? '',
        worker_name: str(row.worker_name) ?? 'unknown',
        config: row.config,
        config_summary: str(row.config_summary),
      }
    })
    .filter((t): t is RegisteredTrigger => t !== null)
}

export interface InvokeOutcome {
  ok: boolean
  durationMs: number
  data?: unknown
  error?: string
}

/** Call a function the way any bus client would; never throws. */
export async function invoke(
  host: Host,
  functionId: string,
  payload: Record<string, unknown>,
): Promise<InvokeOutcome> {
  const started = performance.now()
  try {
    const data = await host.iii.trigger(functionId, payload)
    return { ok: true, durationMs: performance.now() - started, data }
  } catch (err) {
    return {
      ok: false,
      durationMs: performance.now() - started,
      error: errorMessage(err),
    }
  }
}

/** Unique per mount so two mounted pages never share a handler name. */
let hubSeq = 0

/**
 * Reload when the worker fleet changes, instead of polling.
 *
 * The engine publishes no "function registered" event, so the closest true
 * signal is the `worker` trigger type (worker manager add/remove, every
 * lifecycle stage) — the case where the catalogue actually changes under an
 * open tab. Bursts are debounced; everything else is the page's refresh
 * control. The binding filters on BOTH `operations` and `stages`: omitting
 * either matches no events.
 */
export function useFleetChanges(host: Host, reload: () => void) {
  const reloadRef = useRef(reload)
  reloadRef.current = reload
  const handlerId = useMemo(() => {
    hubSeq += 1
    return `iii::console-catalog::fleet-${hubSeq}`
  }, [])

  useEffect(() => {
    let timer: number | null = null
    const schedule = () => {
      if (timer !== null) window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        timer = null
        reloadRef.current()
      }, 400)
    }

    let offHandler: (() => void) | undefined
    let offTrigger: (() => void) | undefined
    try {
      offHandler = host.iii.on(handlerId, schedule)
      offTrigger = host.iii.registerTrigger({
        type: 'worker',
        function_id: `${handlerId}::${host.iii.browserId}`,
        config: {
          operations: ['add', 'remove'],
          stages: ['started', 'downloading', 'downloaded', 'done', 'failed'],
        },
      })
    } catch {
      // No `worker` trigger type on this engine: the refresh control stands
      // in, the page still works.
      offTrigger?.()
      offHandler?.()
      offTrigger = undefined
      offHandler = undefined
    }

    return () => {
      if (timer !== null) window.clearTimeout(timer)
      offTrigger?.()
      offHandler?.()
    }
  }, [host, handlerId])
}

export interface Resource<T> {
  data: T | null
  error: string | null
  loading: boolean
  reload: () => void
}

/**
 * Load `work` and keep the result, with the staleness guard both pages need:
 * a selection changed mid-flight discards the older answer instead of
 * painting it over the newer one. `work` must be a stable callback (the
 * caller's `useCallback`) — it is the dependency.
 */
export function useResource<T>(work: () => Promise<T>): Resource<T> {
  const [data, setData] = useState<T | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [nonce, setNonce] = useState(0)
  const seq = useRef(0)

  useEffect(() => {
    seq.current += 1
    const token = seq.current
    setLoading(true)
    work().then(
      (value) => {
        if (seq.current !== token) return
        setData(value)
        setError(null)
        setLoading(false)
      },
      (err: unknown) => {
        if (seq.current !== token) return
        setError(errorMessage(err))
        setLoading(false)
      },
    )
  }, [work, nonce])

  const reload = useCallback(() => setNonce((n) => n + 1), [])
  return { data, error, loading, reload }
}
