import { describe, expect, it } from 'vitest'
import {
  functionTriggerFromSpan,
  spanFunctionId,
} from './functionTriggerFromSpan'
import type { VisualizationSpan } from './traceTransform'

function vis(overrides: Partial<VisualizationSpan> = {}): VisualizationSpan {
  return {
    name: 'span',
    span_id: 's-1',
    trace_id: 't-1',
    duration_ms: 120,
    status: 'ok',
    depth: 0,
    start_percent: 0,
    width_percent: 100,
    attributes: {},
    events: [],
    links: [],
    pending: false,
    ...overrides,
  }
}

function byId(...spans: VisualizationSpan[]): Map<string, VisualizationSpan> {
  return new Map(spans.map((s) => [s.span_id, s]))
}

describe('spanFunctionId', () => {
  it('prefers the span’s own explicit attributes', () => {
    const span = vis({
      attributes: {
        'faas.invoked_name': 'chat.respond',
        'iii.function.id': 'originator.fn',
      },
    })
    expect(spanFunctionId(span)).toBe('chat.respond')
  })

  it('resolves the owning function from the nearest invocation ancestor, not the baggage originator', () => {
    // trigger slow_fn (engine) → execute slow_fn (worker) → inner fetch span.
    // The inner spans carry only the baggage-stamped iii.function.id, which
    // names the trace ORIGINATOR — the bug this resolution order fixes.
    const trigger = vis({
      span_id: 'trigger',
      name: 'trigger slow_fn',
      attributes: { function_id: 'slow_fn' },
    })
    const execute = vis({
      span_id: 'execute',
      parent_span_id: 'trigger',
      name: 'execute slow_fn',
      attributes: { 'iii.function.id': 'parent_fn' },
    })
    const inner = vis({
      span_id: 'inner',
      parent_span_id: 'execute',
      name: 'HTTP POST',
      attributes: { 'iii.function.id': 'parent_fn' },
    })

    const spans = byId(trigger, execute, inner)
    expect(spanFunctionId(inner, spans)).toBe('slow_fn')
    expect(spanFunctionId(execute, spans)).toBe('slow_fn')
  })

  it('resolves a nested worker execute span from its own name, not the top-level trigger', () => {
    // trigger harness::turn (engine, function_id attr) → execute harness::turn
    // (harness worker) → execute worker::list (worker) → inner HTTP span.
    // The SDK's `execute <fn>` spans carry NO identity attributes (only the
    // name and caller-rewritten baggage), and the engine suppresses its
    // `call <fn>` span for worker-routed functions — so the ancestor walk
    // used to skip past both execute spans and land on the trigger span,
    // labeling the call card with the TOP-LEVEL function.
    const trigger = vis({
      span_id: 'trigger',
      name: 'trigger harness::turn',
      attributes: { function_id: 'harness::turn' },
    })
    const executeTurn = vis({
      span_id: 'execute-turn',
      parent_span_id: 'trigger',
      name: 'execute harness::turn',
      attributes: { 'iii.function.id': 'harness::turn' },
    })
    const executeList = vis({
      span_id: 'execute-list',
      parent_span_id: 'execute-turn',
      name: 'execute worker::list',
      attributes: { 'iii.function.id': 'worker::list' },
    })
    const inner = vis({
      span_id: 'inner',
      parent_span_id: 'execute-list',
      name: 'HTTP GET',
      attributes: {},
    })

    const spans = byId(trigger, executeTurn, executeList, inner)
    expect(spanFunctionId(executeList, spans)).toBe('worker::list')
    expect(spanFunctionId(inner, spans)).toBe('worker::list')
    expect(spanFunctionId(executeTurn, spans)).toBe('harness::turn')
  })

  it('falls back to baggage iii.function.id when no ancestor names a function', () => {
    const orphan = vis({
      parent_span_id: 'missing',
      attributes: { 'iii.function.id': 'parent_fn' },
    })
    expect(spanFunctionId(orphan, byId(orphan))).toBe('parent_fn')
  })

  it('survives a malformed parent cycle', () => {
    const a = vis({
      span_id: 'a',
      parent_span_id: 'b',
      attributes: { 'iii.function.id': 'bag.fn' },
    })
    const b = vis({ span_id: 'b', parent_span_id: 'a', attributes: {} })
    expect(spanFunctionId(a, byId(a, b))).toBe('bag.fn')
  })

  it('returns null when nothing identifies a function', () => {
    expect(spanFunctionId(vis())).toBeNull()
  })
})

describe('functionTriggerFromSpan', () => {
  it('marks a pending invocation span as running without a duration', () => {
    const live = vis({
      pending: true,
      status: 'unset',
      duration_ms: 1_500,
      attributes: { 'faas.invoked_name': 'slow_fn' },
    })
    const call = functionTriggerFromSpan(live)
    expect(call?.functionId).toBe('slow_fn')
    expect(call?.running).toBe(true)
    expect(call?.durationMs).toBeUndefined()
    expect(call?.output).toBeUndefined()
  })

  it('keeps duration and output for finished spans', () => {
    const call = functionTriggerFromSpan(
      vis({ attributes: { function_id: 'chat.respond' } }),
    )
    expect(call?.functionId).toBe('chat.respond')
    expect(call?.running).toBeUndefined()
    expect(call?.durationMs).toBe(120)
  })

  it('renders no card for a span that only inherits its identity and has no payloads', () => {
    // A scope/plumbing span inside an invocation: identity comes from the
    // ancestor walk (or baggage), but the SDK's payload events live on the
    // invocation span — a card here would just show request/response "empty".
    const execute = vis({
      span_id: 'execute',
      name: 'execute worker::list',
    })
    const inner = vis({
      span_id: 'inner',
      parent_span_id: 'execute',
      name: 'HTTP GET',
      attributes: { 'iii.function.id': 'worker::list' },
    })
    expect(functionTriggerFromSpan(inner, byId(execute, inner))).toBeNull()
  })

  it('keeps the card for an inherited-identity span that captured payload data, flagged as inherited', () => {
    const execute = vis({
      span_id: 'execute',
      name: 'execute worker::list',
    })
    const inner = vis({
      span_id: 'inner',
      parent_span_id: 'execute',
      name: 'tool call',
      attributes: { 'tool.arguments': '{"query":"x"}' },
    })
    const call = functionTriggerFromSpan(inner, byId(execute, inner))
    expect(call?.functionId).toBe('worker::list')
    expect(call?.input).toEqual({ query: 'x' })
    // The id names the enclosing invocation, not this sub-span — the card
    // must not claim "triggered ƒ worker::list" off the sub-span's timing.
    expect(call?.identityInherited).toBe(true)
  })

  it('keeps the card for an inherited-identity span that errored, flagged as inherited', () => {
    const execute = vis({
      span_id: 'execute',
      name: 'execute worker::list',
    })
    const inner = vis({
      span_id: 'inner',
      parent_span_id: 'execute',
      name: 'HTTP GET',
      status: 'error',
      events: [
        {
          name: 'exception',
          timestamp_unix_nano: 1,
          attributes: { 'exception.message': 'boom' },
        },
      ],
    })
    const call = functionTriggerFromSpan(inner, byId(execute, inner))
    expect(call?.output).toEqual({ error: 'boom' })
    expect(call?.identityInherited).toBe(true)
  })

  it('flags a pending inherited-identity span too', () => {
    const execute = vis({
      span_id: 'execute',
      name: 'execute worker::list',
    })
    const inner = vis({
      span_id: 'inner',
      parent_span_id: 'execute',
      name: 'tool call',
      pending: true,
      attributes: { 'tool.arguments': '{"query":"x"}' },
    })
    const call = functionTriggerFromSpan(inner, byId(execute, inner))
    expect(call?.running).toBe(true)
    expect(call?.identityInherited).toBe(true)
  })

  it('still renders an empty card for a true invocation span, not flagged', () => {
    // The invocation span is the one place "empty" is honest — payload
    // capture may be disabled (III_DISABLE_TRACE_PAYLOADS) yet the call
    // itself is real and worth a card.
    const call = functionTriggerFromSpan(
      vis({ name: 'execute worker::list', attributes: {} }),
    )
    expect(call?.functionId).toBe('worker::list')
    expect(call?.input).toBeUndefined()
    expect(call?.identityInherited).toBeUndefined()
  })
})
