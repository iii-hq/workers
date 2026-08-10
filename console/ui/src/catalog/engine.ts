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
  const seen = new Set<object>()
  const describe = (value: unknown): string => {
    if (value === null || value === undefined) return 'unknown error'
    if (value instanceof Error) return value.message || value.name
    if (typeof value !== 'object') return String(value)
    if (seen.has(value)) return 'unknown error'
    seen.add(value)

    const record = value as Record<string, unknown>
    const code = typeof record.code === 'string' ? record.code : undefined
    const candidates = [
      record.message,
      record.error,
      record.reason,
      record.detail,
    ]
    for (const candidate of candidates) {
      if (typeof candidate === 'string' && candidate.trim()) {
        return code ? `${code}: ${candidate}` : candidate
      }
      if (candidate && typeof candidate === 'object') {
        const nested = describe(candidate)
        if (nested !== 'unknown error') {
          return code ? `${code}: ${nested}` : nested
        }
      }
    }

    try {
      const serialized = JSON.stringify(value)
      if (serialized && serialized !== '{}') return serialized
    } catch {
      // Cyclic or otherwise non-serializable errors fall through to a stable
      // message instead of leaking the unhelpful default object coercion.
    }
    return code || 'unknown error'
  }
  return describe(err)
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

/** The slice of `engine::workers::list` the pages read: name → runtime. */
export interface WorkerMeta {
  name: string
  /** Worker runtime as reported on the handshake (`node`, `python`, `rust`). */
  runtime?: string
}

export async function listWorkers(host: Host): Promise<WorkerMeta[]> {
  const out = await host.iii.trigger('engine::workers::list', {})
  return rows(out, 'workers')
    .map((row): WorkerMeta | null => {
      if (!isRecord(row)) return null
      const name = str(row.name)
      if (!name) return null
      return { name, runtime: str(row.runtime) }
    })
    .filter((w): w is WorkerMeta => w !== null)
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
 * The engine's own catalogue signals. Both are internal trigger types the
 * engine publishes itself, which is why the pages never poll:
 *
 * - `engine::functions-available` fires when functions are registered or
 *   unregistered (a worker connecting registers its whole surface at once)
 * - `engine::workers-available` fires when a worker connects or disconnects
 * - `trace` is a coalesced "spans changed" tick carrying the affected trace
 *   ids; it is a refetch beat, not a span feed, so a live view re-reads
 *   `engine::traces::list` when it ticks
 */
export type LiveSignal =
  | 'engine::functions-available'
  | 'engine::workers-available'
  | 'trace'

/**
 * Subscribe to engine signals for this component's lifetime and call `onTick`
 * when any of them fires, debounced across bursts (a worker connecting emits
 * one event per function).
 *
 * The binding is a per-tab handler under the `iii::` prefix, which keeps the
 * per-event invocations span-suppressed and out of the trace feed — a live
 * view of traces must not feed itself. It is GC'd with the tab like any
 * Message-path trigger. A missing trigger type degrades to the page's manual
 * refresh rather than breaking the page.
 */
export function useLiveSignals(
  host: Host,
  signals: readonly LiveSignal[],
  onTick: () => void,
  options: { debounceMs?: number } = {},
) {
  const tickRef = useRef(onTick)
  tickRef.current = onTick
  const debounceMs = options.debounceMs ?? 400
  const handlerId = useMemo(() => {
    hubSeq += 1
    return `iii::console-catalog::live-${hubSeq}`
  }, [])
  const key = signals.join(',')

  useEffect(() => {
    let timer: number | null = null
    const schedule = () => {
      if (timer !== null) window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        timer = null
        tickRef.current()
      }, debounceMs)
    }

    const offHandler = host.iii.on(handlerId, schedule)
    const offTriggers = key
      .split(',')
      .map((type) => {
        try {
          return host.iii.registerTrigger({
            type,
            function_id: `${handlerId}::${host.iii.browserId}`,
            config: {},
          })
        } catch {
          return null
        }
      })
      .filter((off): off is () => void => off !== null)

    return () => {
      if (timer !== null) window.clearTimeout(timer)
      for (const off of offTriggers) off()
      offHandler()
    }
  }, [host, handlerId, key, debounceMs])
}

/** One recorded invocation of a function, read back from its span. */
export interface CallRecord {
  spanId: string
  traceId: string
  functionId: string
  startedAtMs: number
  durationMs: number
  ok: boolean
  input?: unknown
  output?: unknown
  worker: string
}

function eventPayload(span: Record<string, unknown>, name: string): unknown {
  const events = Array.isArray(span.events) ? span.events : []
  for (const event of events) {
    if (!isRecord(event) || event.name !== name) continue
    const attrs = Array.isArray(event.attributes) ? event.attributes : []
    for (const attr of attrs) {
      if (!Array.isArray(attr) || attr[0] !== 'iii.payload.json') continue
      try {
        return JSON.parse(String(attr[1]))
      } catch {
        return attr[1]
      }
    }
  }
  return undefined
}

/**
 * Recent calls of one function, newest first.
 *
 * Span names are `execute <function_id>`, so the engine can filter server
 * side instead of the page pulling the whole feed and discarding most of it.
 */
export async function listCalls(
  host: Host,
  functionId: string,
  limit = 25,
): Promise<CallRecord[]> {
  const out = await host.iii.trigger('engine::traces::list', {
    name: `execute ${functionId}`,
    limit,
    include_internal: true,
  })
  return rows(out, 'spans')
    .map((span): CallRecord | null => {
      if (!isRecord(span)) return null
      const start = Number(span.start_time_unix_nano)
      const end = Number(span.end_time_unix_nano)
      if (!Number.isFinite(start)) return null
      return {
        spanId: str(span.span_id) ?? '',
        traceId: str(span.trace_id) ?? '',
        functionId,
        startedAtMs: start / 1e6,
        // In-flight spans carry a null end (Number(null) === 0) — same guard
        // as spansFromFrame, or the duration goes negative.
        durationMs:
          Number.isFinite(end) && end > start ? (end - start) / 1e6 : 0,
        ok: str(span.status) !== 'error',
        input: eventPayload(span, 'iii.invocation.input'),
        output: eventPayload(span, 'iii.invocation.output'),
        worker: str(span.service_name) ?? 'unknown',
      }
    })
    .filter((c): c is CallRecord => c !== null)
    .sort((a, b) => b.startedAtMs - a.startedAtMs)
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

/* ---------------- live span feed ---------------- */

/** One executed call, as the all-spans stream delivers it. */
export interface SpanEvent {
  functionId: string
  worker: string
  durationMs: number
  ok: boolean
  atMs: number
}

/**
 * The engine pushes every non-internal span onto the
 * `iii:devtools:all-spans` stream each coalesce window — the same feed the
 * traces masthead rides. Frame envelope: `{event:{event:{data:{spans}}}}`
 * for the event variant, one level shallower for create/update
 * (`console/web/src/lib/traces-stream.ts` is the reference reader).
 */
function spansFromFrame(frame: unknown): SpanEvent[] {
  if (!isRecord(frame)) return []
  const outer = isRecord(frame.event) ? frame.event : null
  if (!outer) return []
  const inner = isRecord(outer.event) ? outer.event : outer
  const data = isRecord(inner.data) ? inner.data : null
  const spans = data && Array.isArray(data.spans) ? data.spans : []
  const out: SpanEvent[] = []
  for (const span of spans) {
    if (!isRecord(span)) continue
    const name = str(span.name) ?? ''
    // Execution spans are `execute <function_id>`; caller-side `call …`
    // spans would double-count the same invocation.
    if (!name.startsWith('execute ')) continue
    const start = Number(span.start_time_unix_nano)
    // In-flight spans stream with a null end; Number(null) is 0, which would
    // read as a negative duration. Only a real end after the start counts.
    const end = Number(span.end_time_unix_nano)
    if (!Number.isFinite(start) || start <= 0) continue
    out.push({
      functionId: name.slice('execute '.length),
      worker: str(span.service_name) ?? 'unknown',
      durationMs: Number.isFinite(end) && end > start ? (end - start) / 1e6 : 0,
      ok: str(span.status) !== 'error',
      atMs: start / 1e6,
    })
  }
  return out
}

/**
 * Live feed of executed calls for this component's lifetime. Batches arrive
 * as the engine coalesces them (sub-second under load); the binding is a
 * per-tab stream subscription GC'd with the tab.
 */
export function useSpanFeed(host: Host, onSpans: (spans: SpanEvent[]) => void) {
  const handlerRef = useRef(onSpans)
  handlerRef.current = onSpans
  const handlerId = useMemo(() => {
    hubSeq += 1
    return `iii::console-catalog::spans-${hubSeq}`
  }, [])

  useEffect(() => {
    const offHandler = host.iii.on(handlerId, (frame: unknown) => {
      const spans = spansFromFrame(frame)
      if (spans.length > 0) handlerRef.current(spans)
    })
    let offTrigger: (() => void) | undefined
    try {
      offTrigger = host.iii.registerTrigger({
        type: 'stream',
        function_id: `${handlerId}::${host.iii.browserId}`,
        config: { stream_name: 'iii:devtools:all-spans', group_id: 'all' },
      })
    } catch {
      // No stream worker on this engine: pages degrade to manual refresh.
    }
    return () => {
      offTrigger?.()
      offHandler()
    }
  }, [host, handlerId])
}
