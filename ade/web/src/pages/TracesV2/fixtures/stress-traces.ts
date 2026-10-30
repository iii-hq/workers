/**
 * Synthetic LARGE traces for the stress harness (`pnpm test:stress`, and the
 * stress page at /stress.html). Real traces of a few thousand spans are
 * what pushed the page's CPU and allocation up; this generator makes them
 * on demand, deterministically, so the measurement is repeatable.
 *
 * Shape: one `harness::turn step` root, then a breadth-first tree of
 * worker calls with realistic names (`execute router::chat`,
 * `provider::anthropic::stream`, `execute state::get`, …) across eight
 * workers. Siblings are mostly sequential with some genuinely concurrent
 * runs, so the hierarchical timeline packs onto many lines and the
 * waterfall nests several levels deep — both layouts get exercised, not
 * just long flat lists. Every span carries the identity attributes the
 * views read (`iii.function.id`, `iii.session.id`, …) and two captured
 * payload events, sized by `payloadBytes`, so the span panel has something
 * to render too. Roughly one span in eighty fails.
 *
 * Timestamps are MILLISECONDS (`toMs` auto-detects, like the lab fixtures).
 * `snapshotAt` cuts the trace at an instant, returning the spans that had
 * started by then with the unfinished ones marked pending — the stream the
 * page sees while a turn is still running.
 */

import type { SpanEvent, StoredSpan } from '../api/traces'

export interface StressTraceOptions {
  /** Total spans, root included. */
  spanCount: number
  /** PRNG seed — the same seed always yields the same trace. */
  seed?: number
  traceId?: string
  /** Wall-clock start (ms). Defaults to a fixed instant in 2025. */
  startMs?: number
  /** Root span duration (ms). Default 30s. */
  durationMs?: number
  /** Deepest nesting level. Default 6. */
  maxDepth?: number
  /** Approximate size of each captured payload. Default 600 bytes. */
  payloadBytes?: number
  sessionId?: string
}

export const STRESS_TRACE_ID = 'stress-trace-0001'
export const STRESS_T0 = 1_736_500_000_000

/** mulberry32 — small, fast, deterministic. */
function prng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

interface Call {
  name: string
  worker: string
  functionId: string
  kind: string
  llm?: boolean
}

const FIRST_LEVEL: Call[] = [
  {
    name: 'execute router::chat',
    worker: 'llm-router',
    functionId: 'router::chat',
    kind: 'internal',
    llm: true,
  },
  {
    name: 'execute directory::search_functions',
    worker: 'iii-directory',
    functionId: 'directory::search_functions',
    kind: 'internal',
  },
  {
    name: 'execute judge::evaluate',
    worker: 'judge',
    functionId: 'judge::evaluate',
    kind: 'internal',
  },
  {
    name: 'execute state::get',
    worker: 'state',
    functionId: 'state::get',
    kind: 'internal',
  },
  {
    name: 'execute state::set',
    worker: 'state',
    functionId: 'state::set',
    kind: 'internal',
  },
  {
    name: 'execute console::workspace::get',
    worker: 'ade',
    functionId: 'console::workspace::get',
    kind: 'internal',
  },
  {
    name: 'execute ide::fs::read',
    worker: 'ide',
    functionId: 'ide::fs::read',
    kind: 'internal',
  },
  {
    name: 'execute compose::status',
    worker: 'compose',
    functionId: 'compose::status',
    kind: 'internal',
  },
]

const NESTED: Call[] = [
  {
    name: 'execute provider::anthropic::stream',
    worker: 'provider-anthropic',
    functionId: 'provider::anthropic::stream',
    kind: 'client',
    llm: true,
  },
  {
    name: 'execute router::count_tokens',
    worker: 'llm-router',
    functionId: 'router::count_tokens',
    kind: 'internal',
    llm: true,
  },
  {
    name: 'execute session::append',
    worker: 'session-manager',
    functionId: 'session::append',
    kind: 'internal',
  },
  {
    name: 'execute queue::publish',
    worker: 'queue',
    functionId: 'queue::publish',
    kind: 'producer',
  },
  {
    name: 'execute state::get',
    worker: 'state',
    functionId: 'state::get',
    kind: 'internal',
  },
  {
    name: 'execute judge-typesafe::evaluate',
    worker: 'judge-typesafe',
    functionId: 'judge-typesafe::evaluate',
    kind: 'internal',
  },
  {
    name: 'POST /v1/messages',
    worker: 'provider-anthropic',
    functionId: 'provider::anthropic::stream',
    kind: 'client',
  },
  {
    name: 'execute ide::fs::read',
    worker: 'ide',
    functionId: 'ide::fs::read',
    kind: 'internal',
  },
]

const FILLER =
  'Use for general questions and development tasks in your project. The response language is English; keep identifiers and quoted content as they are. '

function payloadJson(
  rand: () => number,
  call: Call,
  bytes: number,
  direction: 'input' | 'output',
): string {
  const text = FILLER.repeat(Math.max(1, Math.ceil(bytes / FILLER.length)))
  const body =
    direction === 'input'
      ? call.llm
        ? {
            _caller_worker_id: 'stress-caller-worker',
            model: 'claude-sonnet-5-5',
            provider: 'anthropic',
            max_output_tokens: 4096,
            messages: [
              { role: 'user', content: [{ type: 'text', text }], timestamp: 1 },
            ],
          }
        : {
            _caller_worker_id: 'stress-caller-worker',
            id: `item-${Math.floor(rand() * 1e6).toString(36)}`,
            query: text.slice(0, Math.max(16, bytes)),
          }
      : call.llm
        ? {
            model: 'claude-sonnet-5-5',
            provider: 'anthropic',
            ok: true,
            stop_reason: 'end',
            usage: {
              input: 1289,
              output: 14,
              cache_read: 0,
              cache_write: 2400,
            },
          }
        : { ok: true, result: text.slice(0, Math.max(16, bytes)) }
  return JSON.stringify(body)
}

interface Slot {
  parentId?: string
  depth: number
  start: number
  end: number
  call: Call
}

/** Build a deterministic trace of `spanCount` spans. */
export function generateStressTrace(options: StressTraceOptions): StoredSpan[] {
  const {
    spanCount,
    seed = 7,
    traceId = STRESS_TRACE_ID,
    startMs = STRESS_T0,
    durationMs = 30_000,
    maxDepth = 6,
    payloadBytes = 600,
    sessionId = 'console-stress-session',
  } = options
  const rand = prng(seed)
  const spans: StoredSpan[] = []
  const slots: Slot[] = [
    {
      depth: 0,
      start: startMs,
      end: startMs + durationMs,
      call: {
        name: 'harness::turn step',
        worker: 'harness',
        functionId: 'harness::send',
        kind: 'internal',
      },
    },
  ]
  const makeId = () => `s${spans.length.toString(36).padStart(5, '0')}`
  const byId = new Map<string, StoredSpan>()

  const emit = (slot: Slot): StoredSpan => {
    const id = makeId()
    const error = slot.depth > 0 && rand() < 1 / 80
    // Integer milliseconds, at least one wide, and never outside the parent
    // once both are rounded (a sub-millisecond slot would otherwise round
    // to zero length or spill a millisecond past its parent).
    const parent = slot.parentId ? byId.get(slot.parentId) : undefined
    let start = Math.round(slot.start)
    let end = Math.max(Math.round(slot.end), start + 1)
    if (parent) {
      end = Math.min(end, parent.end_time_unix_nano)
      start = Math.max(parent.start_time_unix_nano, Math.min(start, end - 1))
    }
    const events: SpanEvent[] = [
      {
        name: 'iii.invocation.input',
        timestamp_unix_nano: start,
        attributes: {
          'iii.payload.json': payloadJson(
            rand,
            slot.call,
            payloadBytes,
            'input',
          ),
        },
      },
      {
        name: 'iii.invocation.output',
        timestamp_unix_nano: end,
        attributes: {
          'iii.payload.json': payloadJson(
            rand,
            slot.call,
            payloadBytes,
            'output',
          ),
        },
      },
    ]
    if (error) {
      events.push({
        name: 'exception',
        timestamp_unix_nano: end,
        attributes: {
          'exception.type': 'TimeoutError',
          'exception.message': `${slot.call.functionId} did not answer in time`,
        },
      })
    }
    const attributes: Array<[string, unknown]> = [
      ['iii.function.id', slot.call.functionId],
      ['iii.session.id', sessionId],
      ['iii.session.name', 'stress session'],
      ['iii.message.id', 'stress-message-1'],
      ['iii.tag.message', 'stress test: render a very large trace'],
    ]
    if (slot.call.name.startsWith('execute ')) {
      attributes.push(['faas.invoked_name', slot.call.functionId])
    }
    if (slot.call.llm) {
      attributes.push(
        ['gen_ai.system', 'anthropic'],
        ['gen_ai.request.model', 'claude-sonnet-5-5'],
        ['gen_ai.usage.input_tokens', 1289],
        ['gen_ai.usage.output_tokens', 14],
      )
    }
    const span: StoredSpan = {
      trace_id: traceId,
      span_id: id,
      parent_span_id: slot.parentId,
      name: slot.call.name,
      kind: slot.call.kind,
      service_name: slot.call.worker,
      start_time_unix_nano: start,
      end_time_unix_nano: end,
      status: error ? 'ERROR' : 'OK',
      attributes,
      events,
      links: [],
    }
    spans.push(span)
    byId.set(id, span)
    return span
  }

  // How many children a span gets: the root fans out wide so a big budget
  // has somewhere to go; below it, mostly a few, sometimes none (a leaf).
  const fanout = (depth: number, remaining: number): number => {
    if (depth === 0)
      return Math.min(remaining, Math.max(4, Math.ceil(spanCount / 40)))
    if (depth >= maxDepth) return 0
    const r = rand()
    const k = r < 0.3 ? 0 : r < 0.55 ? 1 : r < 0.8 ? 2 : r < 0.93 ? 3 : 5
    return Math.min(k, remaining)
  }

  // Children split the parent's time into sequential segments; one in four
  // runs over into the next segment (concurrent siblings), and each child
  // keeps a little slack at both ends so the cascade indents visibly.
  const schedule = (slot: Slot, k: number): Slot[] => {
    const out: Slot[] = []
    const span = slot.end - slot.start
    const pad = Math.min(span * 0.04, 40)
    const inner = Math.max(span - pad * 2, 1)
    const seg = inner / k
    for (let i = 0; i < k; i++) {
      const base = slot.start + pad + seg * i
      const jitter = seg * rand() * 0.15
      const concurrent = rand() < 0.25 && i < k - 1
      const length = concurrent
        ? seg * (1.2 + rand() * 0.6)
        : seg * (0.55 + rand() * 0.4)
      const start = base + jitter
      const end = Math.min(start + length, slot.end - pad)
      const pool = slot.depth === 0 ? FIRST_LEVEL : NESTED
      out.push({
        parentId: undefined, // filled by the caller once the parent has an id
        depth: slot.depth + 1,
        start,
        end: Math.max(end, start + 0.5),
        call: pool[Math.floor(rand() * pool.length)],
      })
    }
    return out
  }

  while (slots.length > 0 && spans.length < spanCount) {
    const slot = slots.shift() as Slot
    const span = emit(slot)
    const remaining = spanCount - spans.length - slots.length
    const k = fanout(slot.depth, Math.max(0, remaining))
    if (k > 0) {
      const window: Slot = {
        ...slot,
        start: span.start_time_unix_nano,
        end: span.end_time_unix_nano,
      }
      for (const child of schedule(window, k)) {
        child.parentId = span.span_id
        slots.push(child)
      }
    }
  }
  // The tree stopped short (every branch went leaf): hang the rest as short
  // sequential leaves under random non-root spans that still have depth.
  while (spans.length < spanCount) {
    const parent =
      spans[1 + Math.floor(rand() * (spans.length - 1))] ?? spans[0]
    const pStart = parent.start_time_unix_nano
    const pEnd = parent.end_time_unix_nano
    const length = Math.max(0.5, (pEnd - pStart) * (0.05 + rand() * 0.2))
    const start = pStart + rand() * Math.max(0, pEnd - pStart - length)
    emit({
      parentId: parent.span_id,
      depth: 2,
      start,
      end: start + length,
      call: NESTED[Math.floor(rand() * NESTED.length)],
    })
  }
  return spans
}

/**
 * The trace as the engine would have served it at `cutoffMs`: spans that
 * had started, with the ones still open marked pending (`end = 0`).
 */
export function snapshotAt(
  spans: readonly StoredSpan[],
  cutoffMs: number,
): StoredSpan[] {
  const out: StoredSpan[] = []
  for (const span of spans) {
    if (span.start_time_unix_nano > cutoffMs) continue
    if (span.end_time_unix_nano > cutoffMs) {
      out.push({ ...span, end_time_unix_nano: 0, pending: true })
    } else {
      out.push(span)
    }
  }
  return out
}

/** Trace-level summary of a generated trace, the way `engine::traces::list` reports it. */
export function summarizeStressTrace(spans: readonly StoredSpan[]) {
  const root = spans[0]
  const errors = spans.filter((s) => s.status === 'ERROR').length
  const pending = spans.some((s) => s.pending)
  const end = Math.max(...spans.map((s) => s.end_time_unix_nano))
  return {
    trace_id: root.trace_id,
    name: root.name,
    start_time_unix_nano: root.start_time_unix_nano,
    ...(pending ? {} : { end_time_unix_nano: end }),
    status: errors > 0 ? 'error' : pending ? 'pending' : 'ok',
    service_name: root.service_name,
    function_id: 'harness::send',
    trace_tags: {
      'iii.session.id': 'console-stress-session',
      'iii.tag.message': 'stress test: render a very large trace',
    },
    span_count: spans.length,
    error_count: errors,
  }
}
