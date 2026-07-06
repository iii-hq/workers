import { describe, expect, it } from 'vitest'
import { functionCallFromSpan, spanFunctionId } from './functionCallFromSpan'
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

describe('functionCallFromSpan', () => {
  it('marks a pending invocation span as running without a duration', () => {
    const live = vis({
      pending: true,
      status: 'unset',
      duration_ms: 1_500,
      attributes: { 'faas.invoked_name': 'slow_fn' },
    })
    const call = functionCallFromSpan(live)
    expect(call?.functionId).toBe('slow_fn')
    expect(call?.running).toBe(true)
    expect(call?.durationMs).toBeUndefined()
    expect(call?.output).toBeUndefined()
  })

  it('keeps duration and output for finished spans', () => {
    const call = functionCallFromSpan(
      vis({ attributes: { function_id: 'chat.respond' } }),
    )
    expect(call?.functionId).toBe('chat.respond')
    expect(call?.running).toBeUndefined()
    expect(call?.durationMs).toBe(120)
  })
})
