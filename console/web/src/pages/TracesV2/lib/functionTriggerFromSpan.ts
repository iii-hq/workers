/**
 * Adapter: OTel span → the shared FunctionTriggerCard's data shape.
 *
 * The card (`components/function-trigger/FunctionTriggerCard`) renders a
 * `FunctionTriggerMessage` — a plain data object — wherever it comes from.
 * Chat builds it from live session events; here we synthesize one from a
 * function-invocation span so the trace detail shows the same call card.
 *
 * Detection: a span is a function invocation when it carries a function id
 * in one of the OTel/iii attributes (`faas.invoked_name`, `function_id`,
 * `iii.function.id`) or is a worker-SDK handler span named `execute <fn>`
 * (those spans carry the function id only in their NAME — see
 * `explicitFunctionId`). A span that only INHERITS an identity (ancestor
 * invocation / baggage) renders a card only when it actually captured
 * payload data — otherwise nested plumbing spans would all show a card
 * with two "empty" panes.
 *
 * Input/output: best effort from the iii-sdk auto-capture events, which
 * store JSON payloads under the `iii.payload.json` event attribute. Event
 * names decide the direction when they can (`*request*`/`*payload*`/
 * `*invocation*` vs `*result*`/`*response*`/`*output*`); otherwise the
 * first payload event is the request and the last distinct one is the
 * response. Falls back to `tool.arguments` for input and the `exception`
 * event for an error output.
 */

import { rawRedactor } from '@/components/function-trigger/renderer-registry'
import type { FunctionTriggerMessage } from '@/types/chat'
import type { FunctionTriggerRenderer } from '@/types/injectable-ui'
import type { VisualizationSpan } from './traceTransform'

const PAYLOAD_ATTR = 'iii.payload.json'
const INPUT_NAME = /payload|request|input|invocation|args/i
const OUTPUT_NAME = /result|response|output|return/i

interface PayloadEvent {
  name: string
  ts: number
  value: unknown
}

/** Parse a JSON-looking string; anything else passes through untouched. */
function parseJsonish(raw: unknown): unknown {
  if (typeof raw !== 'string') return raw
  const trimmed = raw.trim()
  if (trimmed.length === 0) return raw
  const first = trimmed[0]
  if (first !== '{' && first !== '[' && first !== '"') return raw
  try {
    return JSON.parse(trimmed)
  } catch {
    return raw
  }
}

function collectPayloadEvents(span: VisualizationSpan): PayloadEvent[] {
  const out: PayloadEvent[] = []
  for (const event of span.events ?? []) {
    const attrs = event.attributes as Record<string, unknown> | undefined
    if (!attrs || !(PAYLOAD_ATTR in attrs)) continue
    out.push({
      name: event.name ?? '',
      ts: event.timestamp_unix_nano,
      value: parseJsonish(attrs[PAYLOAD_ATTR]),
    })
  }
  out.sort((a, b) => a.ts - b.ts)
  return out
}

function exceptionOutput(span: VisualizationSpan): unknown {
  for (const event of span.events ?? []) {
    if (event.name !== 'exception' && !event.name?.startsWith('exception')) {
      continue
    }
    const attrs = (event.attributes ?? {}) as Record<string, unknown>
    const type = attrs['exception.type']
    const message = attrs['exception.message']
    if (type || message) {
      return { error: [type, message].filter(Boolean).join(': ') }
    }
  }
  return undefined
}

/**
 * The worker SDK names its handler span `execute <function_id>` and sets NO
 * identity attributes on it — the name IS the identity. The prefix was
 * deliberately chosen to be unique to the SDK (engine spans use `trigger `,
 * `call `, `handle_invocation `; user code doesn't name spans `execute X`),
 * the same heuristic philosophy as `spanLabel.ts`.
 */
const EXECUTE_PREFIX = 'execute '

/**
 * Explicit, self-owned function identity: the attributes the engine/SDK set
 * on the invocation span itself (`faas.invoked_name`, `function_id`), or the
 * function id embedded in a worker-SDK `execute <fn>` span name. The
 * baggage-stamped `iii.function.id` is deliberately NOT read here — baggage
 * propagates from the caller, so on child spans it can name the trace
 * originator rather than the function that owns the span.
 *
 * Exported for the trace filter's grouping (`traceTimelineFilters.ts`):
 * a span with an explicit identity is that function's own machinery
 * (queue wrapper, handler span), which is exactly what producer-side
 * `trace_hidden` tagging is meant to hide.
 */
export function explicitFunctionId(
  span: Pick<VisualizationSpan, 'name'> & {
    attributes?: Record<string, unknown>
  },
): string | null {
  const attrs = span.attributes ?? {}
  const id = attrs['faas.invoked_name'] ?? attrs.function_id
  if (typeof id === 'string' && id.length > 0) return id

  if (span.name.startsWith(EXECUTE_PREFIX)) {
    const fromName = span.name.slice(EXECUTE_PREFIX.length).trim()
    if (fromName.length > 0) return fromName
  }
  return null
}

/** Guard against malformed parent chains; real traces are far shallower. */
const MAX_ANCESTOR_WALK = 1000

/**
 * Resolve the function that OWNS a span:
 * 1. the span's own explicit identity — attrs (`faas.invoked_name`,
 *    `function_id`) or an `execute <fn>` worker-handler span name;
 * 2. the nearest ancestor's explicit identity, walking `parent_span_id`
 *    through `spansById` (an invocation span like `trigger <fn>` /
 *    `call <fn>` / `execute <fn>` names every span nested under it —
 *    the engine suppresses its `call <fn>` span for worker-routed
 *    functions, so for nested worker→worker calls the worker's own
 *    `execute <fn>` span is the only invocation boundary);
 * 3. the baggage-stamped `iii.function.id` as a last resort.
 */
export function spanFunctionId(
  span: VisualizationSpan,
  spansById?: Map<string, VisualizationSpan>,
): string | null {
  const own = explicitFunctionId(span)
  if (own) return own

  if (spansById) {
    const visited = new Set<string>([span.span_id])
    let parentId = span.parent_span_id
    for (let hops = 0; parentId && hops < MAX_ANCESTOR_WALK; hops++) {
      if (visited.has(parentId)) break
      visited.add(parentId)
      const parent = spansById.get(parentId)
      if (!parent) break
      const inherited = explicitFunctionId(parent)
      if (inherited) return inherited
      parentId = parent.parent_span_id
    }
  }

  const baggage = span.attributes?.['iii.function.id']
  return typeof baggage === 'string' && baggage.length > 0 ? baggage : null
}

/**
 * Synthesize a `FunctionTriggerMessage` from a function-invocation span, or
 * null when the span isn't one. Pass `spansById` (all spans of the trace)
 * so nested spans resolve to their owning function via the parent chain.
 */
export function functionTriggerFromSpan(
  span: VisualizationSpan,
  spansById?: Map<string, VisualizationSpan>,
): FunctionTriggerMessage | null {
  const functionId = spanFunctionId(span, spansById)
  if (!functionId) return null

  const payloads = collectPayloadEvents(span)
  const inputEvent =
    payloads.find(
      (p) => INPUT_NAME.test(p.name) && !OUTPUT_NAME.test(p.name),
    ) ?? payloads[0]
  const outputCandidates = payloads.filter((p) => p !== inputEvent)
  const outputEvent =
    [...outputCandidates].reverse().find((p) => OUTPUT_NAME.test(p.name)) ??
    outputCandidates[outputCandidates.length - 1]

  const input =
    inputEvent?.value ?? parseJsonish(span.attributes?.['tool.arguments'])
  const output =
    outputEvent?.value ??
    (span.status === 'error' ? exceptionOutput(span) : undefined)

  // Only the invocation span itself (own explicit identity) always gets a
  // card. A span that merely INHERITS its function id — the ancestor walk
  // or baggage above — is machinery inside that call (scope spans, queue
  // wrappers, HTTP/DB clients); the SDK attaches `iii.payload.json` events
  // to the invocation span only, so an inherited span without captured
  // data would render a card whose panes both read "empty". One that DID
  // capture data keeps its card but is flagged `identityInherited` — its
  // duration measures the sub-span, not the invocation, so the card must
  // not claim "triggering/triggered ƒ <fn>".
  const inherited = explicitFunctionId(span) === null
  if (inherited && input === undefined && output === undefined) {
    return null
  }

  if (span.pending) {
    // Still running: no duration or output yet — the card renders its
    // `running` pulse instead.
    return {
      id: span.span_id,
      role: 'function-trigger',
      createdAt: 0,
      functionId,
      input,
      running: true,
      ...(inherited ? { identityInherited: true } : {}),
    }
  }

  return {
    id: span.span_id,
    role: 'function-trigger',
    createdAt: 0,
    functionId,
    input,
    output,
    durationMs: Math.round(span.duration_ms),
    ...(inherited ? { identityInherited: true } : {}),
  }
}

/**
 * The redactor `SpanTagsTab`/`SpanLogsTab` should apply to a span's own
 * attributes/events, resolved the SAME way `functionTriggerFromSpan` above
 * resolves the info tab's card — `spanFunctionId`, not the narrower
 * `identityInherited` gating this function applies before deciding whether
 * to SHOW a card at all. That gate is about the info tab's card; it is not
 * a reason to under-redact a sibling tab that always renders regardless.
 *
 * `undefined` when `spanFunctionId` resolves no function id (the span is
 * not part of any function invocation) or when no injected renderer's
 * `redactRaw` claims it — the overwhelmingly common case, matching
 * `rawRedactor`'s own contract.
 */
export function spanRawRedactor(
  span: VisualizationSpan,
  spansById: Map<string, VisualizationSpan> | undefined,
  renderers: readonly FunctionTriggerRenderer[],
): ((value: unknown) => unknown) | undefined {
  const functionId = spanFunctionId(span, spansById)
  return functionId ? rawRedactor(renderers, functionId) : undefined
}
