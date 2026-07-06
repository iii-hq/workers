/**
 * Static trace fixtures for the TracesV2 lab (Storybook stories + playground).
 *
 * The real Traces surface has no sample data anywhere — every span arrives
 * over the iii-browser-sdk WebSocket. These fixtures are the missing piece:
 * a hand-authored `StoredSpan[]` (the engine wire shape) plus the view-models
 * derived from it through the SAME pure transforms the app uses
 * (`toWaterfallData`, `mapSpanToListItem`, `dedupeToTraceRoots`). That keeps
 * the fixtures honest — if a transform changes, the derived exports change
 * with it, exactly like production.
 *
 * Timestamps: authored in MILLISECONDS. `toMs()` in traceTransform auto-detects
 * ms vs ns (threshold = Jan 1 2100 in ms), and ms values are well under it, so
 * the field name `_unix_nano` is honored structurally while staying readable.
 * A single fixed base (`T0`) keeps every story deterministic.
 */

import type { OtelLog } from '../api/otel-logs'
import type { SpanEvent, SpanLink, StoredSpan, TraceGroup } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import { dedupeToTraceRoots, mapSpanToListItem } from '../lib/traceListItem'
import {
  toWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from '../lib/traceTransform'

/** Fixed base time (~Jan 2025, in ms) so stories are deterministic. */
export const T0 = 1_736_500_000_000

/** Small helper to build a `StoredSpan` with sane defaults. */
function span(s: {
  trace_id: string
  span_id: string
  parent_span_id?: string
  name: string
  kind?: string
  service_name: string
  start: number
  end: number
  status?: 'OK' | 'ERROR' | 'UNSET'
  attributes?: Array<[string, unknown]>
  events?: SpanEvent[]
  links?: SpanLink[]
}): StoredSpan {
  return {
    trace_id: s.trace_id,
    span_id: s.span_id,
    parent_span_id: s.parent_span_id,
    name: s.name,
    kind: s.kind,
    service_name: s.service_name,
    start_time_unix_nano: s.start,
    end_time_unix_nano: s.end,
    status: s.status ?? 'OK',
    attributes: s.attributes ?? [],
    events: s.events ?? [],
    links: s.links ?? [],
  }
}

// ─────────────────────────────────────────────────────────────────────────
// Trace 1 — rich agent turn. Depth 5, four services, an engine-routing
// pair (handle_invocation/call greet), an LLM client span, an ERRORED tool
// span with an exception event, a db span, baggage + link attributes.
// ─────────────────────────────────────────────────────────────────────────

export const TRACE_1_ID = 'trace-agent-0000000000000001'

export const TRACE_1_SPANS: StoredSpan[] = [
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a1-root',
    name: 'POST /api/chat',
    kind: 'server',
    service_name: 'gateway',
    start: T0,
    end: T0 + 1200,
    attributes: [
      ['http.method', 'POST'],
      ['http.route', '/api/chat'],
      ['http.status_code', 200],
      ['iii.session.id', 'sess-abc123'],
      ['iii.message.id', 'msg-000001'],
      ['iii.function.id', 'chat.respond'],
    ],
  }),
  // Engine routing pair: handle_invocation greet → call greet (function_id set).
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a2-handle',
    parent_span_id: 'a1-root',
    name: 'handle_invocation chat.respond',
    kind: 'internal',
    service_name: 'iii',
    start: T0 + 20,
    end: T0 + 1180,
    attributes: [['function_id', 'chat.respond']],
  }),
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a3-call',
    parent_span_id: 'a2-handle',
    name: 'call chat.respond',
    kind: 'client',
    service_name: 'iii',
    start: T0 + 25,
    end: T0 + 1175,
    attributes: [['function_id', 'chat.respond']],
  }),
  // The actual worker function.
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a4-fn',
    parent_span_id: 'a3-call',
    name: 'chat.respond',
    kind: 'internal',
    service_name: 'agent',
    start: T0 + 30,
    end: T0 + 1170,
    attributes: [
      ['faas.invoked_name', 'chat.respond'],
      ['baggage.session.id', 'sess-abc123'],
      ['baggage.tenant', 'acme'],
      ['code.function', 'respond'],
      ['code.namespace', 'handlers.chat'],
    ],
    // iii-sdk auto-capture: invocation payload + result as span events with
    // `iii.payload.json` attributes. Feeds the FunctionCallCard in the span
    // info tab (see lib/functionCallFromSpan.ts).
    events: [
      {
        name: 'iii.invocation.payload',
        timestamp_unix_nano: T0 + 32,
        attributes: {
          'iii.payload.json': JSON.stringify({
            message: 'summarize the incident timeline',
            session_id: 'sess-abc123',
          }),
        },
      },
      {
        name: 'iii.invocation.result',
        timestamp_unix_nano: T0 + 1168,
        attributes: {
          'iii.payload.json': JSON.stringify({
            reply: 'The incident began at 09:14 UTC…',
            tokens: 342,
          }),
        },
      },
    ],
  }),
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a5-llm',
    parent_span_id: 'a4-fn',
    name: 'llm.completion',
    kind: 'client',
    service_name: 'agent',
    start: T0 + 40,
    end: T0 + 900,
    attributes: [
      ['gen_ai.system', 'anthropic'],
      ['gen_ai.request.model', 'claude-opus-4'],
      ['gen_ai.request.max_tokens', 1024],
      ['gen_ai.usage.input_tokens', 1240],
      ['gen_ai.usage.output_tokens', 342],
      ['gen_ai.response.finish_reason', 'end_turn'],
    ],
    events: [
      {
        name: 'gen_ai.content.prompt',
        timestamp_unix_nano: T0 + 45,
        attributes: { 'gen_ai.prompt': 'summarize the incident timeline' },
      },
      {
        name: 'gen_ai.content.completion',
        timestamp_unix_nano: T0 + 890,
        attributes: { 'gen_ai.completion': 'The incident began at 09:14 UTC…' },
      },
    ],
    links: [
      {
        trace_id: 'trace-http-0000000000000002',
        span_id: 'h1-root',
        attributes: { 'link.rel': 'follows_from' },
      },
    ],
  }),
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a6-tool',
    parent_span_id: 'a4-fn',
    name: 'tool.search_docs',
    kind: 'internal',
    service_name: 'agent',
    start: T0 + 920,
    end: T0 + 1050,
    status: 'ERROR',
    attributes: [
      ['tool.name', 'search_docs'],
      ['tool.arguments', '{"query":"incident runbook"}'],
      ['error', true],
    ],
    events: [
      {
        name: 'exception',
        timestamp_unix_nano: T0 + 1048,
        attributes: {
          'exception.type': 'TimeoutError',
          'exception.message': 'vector store timed out after 120ms',
          'exception.stacktrace':
            'TimeoutError: vector store timed out after 120ms\n    at VectorStore.search (vector.ts:88:11)\n    at searchDocs (tools/search.ts:24:20)',
        },
      },
    ],
  }),
  span({
    trace_id: TRACE_1_ID,
    span_id: 'a7-db',
    parent_span_id: 'a4-fn',
    name: 'db.query',
    kind: 'client',
    service_name: 'postgres',
    start: T0 + 1055,
    end: T0 + 1150,
    attributes: [
      ['db.system', 'postgresql'],
      ['db.name', 'app'],
      ['db.statement', 'SELECT id, title FROM incidents WHERE status = $1'],
      ['db.rows_affected', 3],
    ],
  }),
]

// ─────────────────────────────────────────────────────────────────────────
// Trace 2 — simple healthy request (2 spans, 2 services).
// ─────────────────────────────────────────────────────────────────────────

export const TRACE_2_ID = 'trace-http-0000000000000002'

export const TRACE_2_SPANS: StoredSpan[] = [
  span({
    trace_id: TRACE_2_ID,
    span_id: 'h1-root',
    name: 'GET /healthz',
    kind: 'server',
    service_name: 'gateway',
    start: T0 - 5000,
    end: T0 - 4982,
    attributes: [
      ['http.method', 'GET'],
      ['http.route', '/healthz'],
      ['http.status_code', 200],
    ],
  }),
  span({
    trace_id: TRACE_2_ID,
    span_id: 'h2-auth',
    parent_span_id: 'h1-root',
    name: 'auth.verify',
    kind: 'internal',
    service_name: 'worker-auth',
    start: T0 - 4996,
    end: T0 - 4986,
    attributes: [['faas.invoked_name', 'auth.verify']],
  }),
]

// ─────────────────────────────────────────────────────────────────────────
// Trace 3 — errored queue consumer (root error + topic).
// ─────────────────────────────────────────────────────────────────────────

export const TRACE_3_ID = 'trace-error-0000000000000003'

export const TRACE_3_SPANS: StoredSpan[] = [
  span({
    trace_id: TRACE_3_ID,
    span_id: 'e1-root',
    name: 'orders.process',
    kind: 'consumer',
    service_name: 'worker-orders',
    start: T0 - 2000,
    end: T0 - 1700,
    status: 'ERROR',
    attributes: [
      ['faas.invoked_name', 'orders.process'],
      ['messaging.destination.name', 'orders.queue'],
      ['messaging.system', 'iii'],
      ['error', true],
    ],
    events: [
      {
        name: 'exception',
        timestamp_unix_nano: T0 - 1702,
        attributes: {
          'exception.type': 'ValidationError',
          'exception.message': 'order 8842 missing required field "total"',
        },
      },
    ],
  }),
]

// ─────────────────────────────────────────────────────────────────────────
// Trace 4 — quick internal cron tick.
// ─────────────────────────────────────────────────────────────────────────

export const TRACE_4_ID = 'trace-cron-0000000000000004'

export const TRACE_4_SPANS: StoredSpan[] = [
  span({
    trace_id: TRACE_4_ID,
    span_id: 'c1-root',
    name: 'cron.tick',
    kind: 'internal',
    service_name: 'iii',
    start: T0 - 100,
    end: T0 - 95,
    attributes: [
      ['function_id', 'cron.tick'],
      ['iii.function.id', 'cron.tick'],
    ],
  }),
]

/** Every span across every fixture trace (what a `trace_id`-scoped read returns). */
export const ALL_SPANS: StoredSpan[] = [
  ...TRACE_1_SPANS,
  ...TRACE_2_SPANS,
  ...TRACE_3_SPANS,
  ...TRACE_4_SPANS,
]

/** Root-only spans (what an unscoped `engine::traces::list` returns). */
export const LIST_SPANS: StoredSpan[] = [
  TRACE_1_SPANS[0],
  TRACE_2_SPANS[0],
  TRACE_3_SPANS[0],
  TRACE_4_SPANS[0],
]

// ── Derived view-models (through the real transforms) ──────────────────────

/** The flat trace-list rows, newest first — as `useTraceData` would produce. */
export const TRACE_LIST_FIXTURE: TraceListItem[] = dedupeToTraceRoots(
  LIST_SPANS,
)
  .map(mapSpanToListItem)
  .sort((a, b) => b.startTime - a.startTime)

/** Waterfall/flame data for the rich agent trace. Non-null by construction. */
export const WATERFALL_FIXTURE: WaterfallData = toWaterfallData(
  TRACE_1_SPANS,
  TRACE_1_ID,
) as WaterfallData

/** Waterfall data for the simple healthy trace. */
export const WATERFALL_SIMPLE: WaterfallData = toWaterfallData(
  TRACE_2_SPANS,
  TRACE_2_ID,
) as WaterfallData

/** Look a visualization span up by id (throws if absent — fixtures are fixed). */
export function getSpan(
  data: WaterfallData,
  spanId: string,
): VisualizationSpan {
  const found = data.spans.find((s) => s.span_id === spanId)
  if (!found) throw new Error(`fixture span not found: ${spanId}`)
  return found
}

/** The root span of the rich trace (server, http attrs). */
export const ROOT_SPAN: VisualizationSpan = getSpan(
  WATERFALL_FIXTURE,
  'a1-root',
)
/** The LLM client span (gen_ai attrs, events, a link). */
export const LLM_SPAN: VisualizationSpan = getSpan(WATERFALL_FIXTURE, 'a5-llm')
/** The errored tool span (status error + exception event). */
export const ERROR_SPAN: VisualizationSpan = getSpan(
  WATERFALL_FIXTURE,
  'a6-tool',
)
/** The worker fn span (baggage.* attributes). */
export const FN_SPAN: VisualizationSpan = getSpan(WATERFALL_FIXTURE, 'a4-fn')
/** The db client span. */
export const DB_SPAN: VisualizationSpan = getSpan(WATERFALL_FIXTURE, 'a7-db')

// ── Group-by aggregate (session view / group-by dropdown) ───────────────────

export const TRACE_GROUPS_FIXTURE: TraceGroup[] = [
  {
    value: 'sess-abc123',
    trace_ids: [TRACE_1_ID],
    span_count: TRACE_1_SPANS.length,
    first_seen_ms: T0,
    last_seen_ms: T0 + 1200,
    duration_ms: 1200,
    error_count: 1,
  },
  {
    value: 'sess-def456',
    trace_ids: [TRACE_2_ID, TRACE_4_ID],
    span_count: TRACE_2_SPANS.length + TRACE_4_SPANS.length,
    first_seen_ms: T0 - 5000,
    last_seen_ms: T0 - 95,
    duration_ms: 4905,
    error_count: 0,
  },
]

// ── OTel logs (for the SpanOtelLogsTab / logs surfaces) ─────────────────────

export const OTEL_LOGS_FIXTURE: OtelLog[] = [
  {
    timestamp_unix_nano: T0 + 35,
    observed_timestamp_unix_nano: T0 + 35,
    trace_id: TRACE_1_ID,
    span_id: 'a4-fn',
    severity_number: 9,
    severity_text: 'INFO',
    body: 'chat.respond started for session sess-abc123',
    attributes: { 'log.source': 'handlers.chat' },
    resource: { 'service.name': 'agent' },
    service_name: 'agent',
  },
  {
    timestamp_unix_nano: T0 + 905,
    observed_timestamp_unix_nano: T0 + 905,
    trace_id: TRACE_1_ID,
    span_id: 'a4-fn',
    severity_number: 13,
    severity_text: 'WARN',
    body: 'llm.completion latency 860ms exceeded target 500ms',
    attributes: { 'log.source': 'llm.client', latency_ms: 860 },
    resource: { 'service.name': 'agent' },
    service_name: 'agent',
  },
  {
    timestamp_unix_nano: T0 + 1049,
    observed_timestamp_unix_nano: T0 + 1049,
    trace_id: TRACE_1_ID,
    span_id: 'a6-tool',
    severity_number: 17,
    severity_text: 'ERROR',
    body: 'tool.search_docs failed: vector store timed out after 120ms',
    attributes: { 'log.source': 'tools.search', 'error.kind': 'timeout' },
    resource: { 'service.name': 'agent' },
    service_name: 'agent',
  },
]
