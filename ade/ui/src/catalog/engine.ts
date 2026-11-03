/**
 * The engine catalogue calls behind the Functions and Observers pages, plus
 * the narrow runtime guards that keep an unexpected wire shape from
 * reaching React as `undefined.map`.
 *
 * Wire source: `iii/engine/src/workers/engine_fn/mod.rs` and
 * `iii/engine/src/observer/builtins.rs`. Everything here is read-only
 * except `invoke`, a plain function call over the tab's bus
 * (`host.iii.call`) — the same privilege any worker on the bus has — and
 * `disposeObserver`.
 *
 * Parsing is deliberately permissive: unknown fields pass through, absent
 * optionals stay absent, and only the fields the pages actually render are
 * required. A row the engine grew a field for still lists.
 */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { type SpanEvent, startSpanFeed } from './span-feed'

export interface FunctionSummary {
  function_id: string
  worker_name: string
  description?: string | null
  /** JSON Schema of the args its observers pass; absent when it takes none. */
  args_format?: unknown
  /** Only the worker that hosts it calls it: an event source. */
  private?: boolean
  /** `false` when nobody may observe it. */
  observable?: boolean
  /** Live observers of it, in any state but `disposed`. */
  observer_count?: number
}

export interface FunctionDetail extends FunctionSummary {
  request_schema?: unknown
  response_schema?: unknown
  metadata?: unknown
}

/** An event source: a function its worker calls with the event (private), or one that takes args. */
export function isEventSource(fn: FunctionSummary): boolean {
  return fn.private === true || fn.args_format !== undefined
}

/** One observer, as `engine::observers::list` / `get` report it. */
export interface ObserverInfo {
  id: string
  /** The observed function. */
  function_id: string
  namespace: string
  /** Read by the observed function's worker (a route, a schedule, a topic). */
  args: Record<string, unknown>
  /** Which calls reach the target, normalized. */
  filters: Record<string, unknown>
  target: { function_id: string; namespace: string }
  capture: string[]
  on: string
  deliver: string
  /** The worker that created the observer. */
  worker: { worker_id: string; worker_name?: string }
  state: string
  reason?: { code: string; message: string }
  fires: number
  /** Activations dropped, by reason. */
  dropped: Record<string, number>
  activation_failures: Record<string, number>
  max_fires?: number
  expires_in_ms?: number
  max_in_flight?: number
  created_at_ms?: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function num(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function counts(value: unknown): Record<string, number> {
  if (!isRecord(value)) return {}
  const out: Record<string, number> = {}
  for (const [key, n] of Object.entries(value)) {
    if (typeof n === 'number') out[key] = n
  }
  return out
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

function isFunctionUnavailable(err: unknown): boolean {
  return /function_not_found|function .*not (?:found|registered)|not registered/i.test(
    errorMessage(err),
  )
}

function functionSummary(row: unknown): FunctionSummary | null {
  if (!isRecord(row)) return null
  const function_id = str(row.function_id)
  if (!function_id) return null
  const summary: FunctionSummary = {
    function_id,
    worker_name: str(row.worker_name) ?? 'unknown',
    description: description(row.description),
  }
  if (row.args_format !== undefined && row.args_format !== null) {
    summary.args_format = row.args_format
  }
  if (row.private === true) summary.private = true
  if (row.observable === false) summary.observable = false
  const observers = num(row.observer_count)
  if (observers !== undefined) summary.observer_count = observers
  return summary
}

export function observerInfo(row: unknown): ObserverInfo | null {
  if (!isRecord(row)) return null
  const id = str(row.id)
  const function_id = str(row.function_id)
  if (!id || !function_id) return null
  const target = isRecord(row.target) ? row.target : {}
  const worker = isRecord(row.worker) ? row.worker : {}
  const reason = isRecord(row.reason) ? row.reason : undefined
  return {
    id,
    function_id,
    namespace: str(row.namespace) ?? 'default',
    args: isRecord(row.args) ? row.args : {},
    filters: isRecord(row.filters) ? row.filters : {},
    target: {
      function_id: str(target.function_id) ?? '',
      namespace: str(target.namespace) ?? 'default',
    },
    capture: Array.isArray(row.capture)
      ? row.capture.filter((c): c is string => typeof c === 'string')
      : [],
    on: str(row.on) ?? 'success',
    deliver: str(row.deliver) ?? 'envelope',
    worker: {
      worker_id: str(worker.worker_id) ?? '',
      worker_name: str(worker.worker_name),
    },
    state: str(row.state) ?? 'pending',
    reason: reason
      ? { code: str(reason.code) ?? '', message: str(reason.message) ?? '' }
      : undefined,
    fires: num(row.fires) ?? 0,
    dropped: counts(row.dropped),
    activation_failures: counts(row.activation_failures),
    max_fires: num(row.max_fires),
    expires_in_ms: num(row.expires_in_ms),
    max_in_flight: num(row.max_in_flight),
    created_at_ms: num(row.created_at_ms),
  }
}

export async function listFunctions(
  host: Host,
  options: { includeInternal: boolean },
): Promise<FunctionSummary[]> {
  const out = await host.iii.call('engine::functions::list', {
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
  const out = await host.iii.call('engine::functions::info', {
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
  }
}

/** The slice of `engine::workers::list` the pages read: name → runtime. */
export interface WorkerMeta {
  name: string
  /** Worker runtime as reported on the handshake (`node`, `python`, `rust`). */
  runtime?: string
}

export async function listWorkers(host: Host): Promise<WorkerMeta[]> {
  const out = await host.iii.call('engine::workers::list', {})
  return rows(out, 'workers')
    .map((row): WorkerMeta | null => {
      if (!isRecord(row)) return null
      const name = str(row.name)
      if (!name) return null
      return { name, runtime: str(row.runtime) }
    })
    .filter((w): w is WorkerMeta => w !== null)
}

/** Every observer, or those of one observed function or one observing worker. */
export async function listObservers(
  host: Host,
  filter: { function_id?: string; worker?: string } = {},
): Promise<ObserverInfo[]> {
  const out = await host.iii.call('engine::observers::list', filter)
  return rows(out, 'observers')
    .map(observerInfo)
    .filter((o): o is ObserverInfo => o !== null)
}

export async function getObserver(
  host: Host,
  id: string,
): Promise<ObserverInfo> {
  const out = await host.iii.call('engine::observers::get', { id })
  const observer = observerInfo(isRecord(out) && isRecord(out.observer) ? out.observer : out)
  if (!observer) throw new Error('engine::observers::get returned no observer')
  return observer
}

/** Dispose an observer; resolves `false` when it was already gone. */
export async function disposeObserver(host: Host, id: string): Promise<boolean> {
  const out = await host.iii.call('engine::observers::dispose', { id })
  return isRecord(out) ? out.disposed === true : false
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
    const data = await host.iii.call(functionId, payload)
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
 * The engine's own sources, which is why the pages never poll. Each is a
 * private function the engine calls with the event:
 *
 * - `engine::on-functions-changed` is called when the function set changes,
 *   with the full function list; a burst of registers, re-registers or
 *   unregisters is folded into one call ~100ms after it
 * - `engine::on-workers-changed` is called when a worker connects,
 *   disconnects or updates its metadata
 * - `engine::on-observers-changed` is called when an observer changes state
 * - `observability::on-span` is called with a coalesced "spans changed"
 *   tick carrying the affected trace ids; it is a refetch beat, not a span
 *   feed, so a live view re-reads `engine::traces::spans` when it arrives
 */
export type LiveSource =
  | 'engine::on-functions-changed'
  | 'engine::on-workers-changed'
  | 'engine::on-observers-changed'
  | 'observability::on-span'

/** Engine-hosted sources outside `engine::*` resolve in `default` only when named. */
function liveSourceNamespace(source: LiveSource): string | undefined {
  return source.startsWith('engine::') ? undefined : 'default'
}

/**
 * Observe engine sources for this component's lifetime and call `onTick`
 * when any of them is called, debounced so one change costs one refetch.
 *
 * The engine already folds a burst of function changes into one
 * `engine::on-functions-changed` call, but the debounce still earns its
 * keep: one worker connecting calls that AND `engine::on-workers-changed`,
 * which the engine calls independently; a surface registered over longer
 * than the engine's fold window (or several workers starting together)
 * still arrives as a few calls; and `observability::on-span` keeps ticking
 * under load. Each tick would otherwise be a full catalogue re-read.
 *
 * The target is a per-tab handler under the `iii::` prefix, which keeps the
 * per-event invocations span-suppressed and out of the trace feed — a live
 * view of traces must not feed itself. Its observers are disposed on
 * unmount and go with the tab. A source nobody hosts leaves its observer
 * pending, and the page falls back to its manual refresh.
 */
export function useLiveSignals(
  host: Host,
  sources: readonly LiveSource[],
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
  const key = sources.join(',')

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
    const target = `${handlerId}::${host.iii.browserId}`
    const offObservers = (key.split(',') as LiveSource[])
      .map((source) => {
        try {
          return host.iii.observe(
            {
              function_id: source,
              namespace: liveSourceNamespace(source),
              deliver: 'output',
            },
            target,
          )
        } catch {
          return null
        }
      })
      .filter((off): off is () => void => off !== null)

    return () => {
      if (timer !== null) window.clearTimeout(timer)
      for (const off of offObservers) off()
      offHandler()
    }
  }, [host, handlerId, key, debounceMs])
}

const CALLS_TIMEOUT_MS = 15_000

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
  const payload = {
    name: `execute ${functionId}`,
    limit,
    include_internal: true,
  }
  // A trace store past its disk limit can leave this query unanswered for
  // good; bounded, the page reports it instead of reading forever.
  const options = { timeoutMs: CALLS_TIMEOUT_MS }
  let out: unknown
  try {
    out = await host.iii.call('engine::traces::spans', payload, options)
  } catch (err) {
    if (!isFunctionUnavailable(err)) throw err
    out = await host.iii.call('engine::traces::list', payload, options)
  }
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
        // as the live span feed, or the duration goes negative.
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

export type { SpanEvent } from './span-feed'

/**
 * Live feed of executed calls for this component's lifetime: each coalesced
 * `observability::on-span` tick (ids only) starts a bounded re-read of the
 * touched traces (`engine::traces::spans`), and only calls not delivered yet
 * reach `onSpans` (see `./span-feed`). The target is a per-tab `iii::`
 * handler that goes with the tab; without an observability worker the page
 * degrades to manual refresh.
 */
export function useSpanFeed(host: Host, onSpans: (spans: SpanEvent[]) => void) {
  const handlerRef = useRef(onSpans)
  handlerRef.current = onSpans
  const handlerId = useMemo(() => {
    hubSeq += 1
    return `iii::console-catalog::spans-${hubSeq}`
  }, [])

  useEffect(
    () =>
      startSpanFeed(host.iii, (spans) => handlerRef.current(spans), {
        handlerId,
      }),
    [host, handlerId],
  )
}
