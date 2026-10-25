/**
 * A fake `ISdk` for the stress page: answers the Traces page's engine calls
 * from a generated trace, with no WebSocket and no engine. Where the lab
 * harness (`pages/TracesV2/stories/harness.tsx`) serves hand-authored
 * fixtures, this one serves a trace of any size and can STREAM it: the
 * snapshot advances on demand and the page's trace-activity trigger fires,
 * exactly the notify-then-query loop a live engine runs.
 */

import type { ISdk } from 'iii-browser-sdk'
import type { StoredSpan } from '@/pages/TracesV2/api/traces'
import {
  generateStressTrace,
  snapshotAt,
  summarizeStressTrace,
} from '@/pages/TracesV2/fixtures/stress-traces'

type TriggerArgs = { function_id: string; payload?: Record<string, unknown> }
type Handler = (data: unknown) => Promise<unknown>

export interface StressSdk {
  sdk: ISdk
  /** The whole trace, finished. */
  spans: StoredSpan[]
  /** Move the stream cutoff to `fraction` (0..1) of the trace and fire the
   *  activity tick; resolves once every handler has run. */
  advance(fraction: number): Promise<void>
  /** What the page is currently served. */
  snapshot(): StoredSpan[]
  /** Engine calls answered so far, by function id. */
  calls: Record<string, number>
}

export interface StressSdkOptions {
  spanCount: number
  seed?: number
  /** Start streaming: the trace begins `durationMs` before now and the
   *  snapshot starts at `initialFraction` of it. Off: the trace is served
   *  whole, finished, at a fixed instant in 2025. */
  stream?: boolean
  durationMs?: number
  initialFraction?: number
  payloadBytes?: number
}

export function createStressSdk(options: StressSdkOptions): StressSdk {
  const {
    spanCount,
    seed = 7,
    stream = false,
    durationMs = 30_000,
    initialFraction = 0.3,
    payloadBytes,
  } = options
  const startMs = stream ? Date.now() - durationMs * initialFraction : undefined
  const spans = generateStressTrace({
    spanCount,
    seed,
    durationMs,
    payloadBytes,
    ...(startMs !== undefined ? { startMs } : {}),
  })
  const traceStart = spans[0].start_time_unix_nano
  let served: StoredSpan[] = stream
    ? snapshotAt(spans, traceStart + durationMs * initialFraction)
    : spans
  const handlers = new Map<string, Handler>()
  const calls: Record<string, number> = {}

  const page = <T>(items: T[], p: Record<string, unknown>) => {
    const offset = (p.offset as number) ?? 0
    const limit = (p.limit as number) ?? 100
    return {
      spans: items.slice(offset, offset + limit),
      total: items.length,
      offset,
      limit,
    }
  }

  async function trigger({
    function_id,
    payload,
  }: TriggerArgs): Promise<unknown> {
    calls[function_id] = (calls[function_id] ?? 0) + 1
    const p = payload ?? {}
    switch (function_id) {
      case 'engine::traces::list': {
        const summary = summarizeStressTrace(served)
        const items = [summary]
        return {
          traces: items,
          total: 1,
          offset: 0,
          limit: (p.limit as number) ?? 100,
        }
      }
      case 'engine::traces::spans': {
        const traceId = p.trace_id as string | undefined
        const ids = p.trace_ids as string[] | undefined
        let items = served
        if (traceId) items = items.filter((s) => s.trace_id === traceId)
        if (ids) items = items.filter((s) => ids.includes(s.trace_id))
        if (p.include_events === false) {
          items = items.map((s) => ({ ...s, events: [] }))
        }
        return page(items, p)
      }
      case 'engine::traces::group_by':
        return { groups: [] }
      case 'engine::logs::list':
        return { logs: [], total: 0 }
      case 'configuration::get':
        return {}
      default:
        return {}
    }
  }

  const sdk = {
    trigger: (args: TriggerArgs) => trigger(args),
    registerFunction: (id: string, handler: Handler) => {
      handlers.set(id, handler)
      return { unregister: () => handlers.delete(id) }
    },
    registerTrigger: () => ({ unregister() {} }),
    addConnectionStateListener: () => () => {},
    shutdown: async () => {},
  } as unknown as ISdk

  return {
    sdk,
    spans,
    calls,
    snapshot: () => served,
    async advance(fraction) {
      const cutoff =
        traceStart + durationMs * Math.min(1, Math.max(0, fraction))
      served = fraction >= 1 ? spans : snapshotAt(spans, cutoff)
      const ticks: Promise<unknown>[] = []
      for (const [id, handler] of handlers) {
        if (id.startsWith('iii::console::trace_activity')) {
          ticks.push(handler({ trace_ids: [spans[0].trace_id] }))
        }
      }
      await Promise.allSettled(ticks)
    },
  }
}
