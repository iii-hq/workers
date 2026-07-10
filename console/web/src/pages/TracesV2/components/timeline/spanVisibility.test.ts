import { describe, expect, it } from 'vitest'
import { traceSpanGroupKey } from '../../lib/traceTimelineFilters'
import type { VisualizationSpan, WaterfallData } from '../../lib/traceTransform'
import {
  applyHiddenSpanFilters,
  deriveSpanGroups,
  reparentThroughHidden,
  workerGroupKey,
} from './spanVisibility'

function vis(overrides: Partial<VisualizationSpan> = {}): VisualizationSpan {
  return {
    name: 'span',
    span_id: 's-1',
    trace_id: 't-1',
    duration_ms: 10,
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

function waterfall(spans: VisualizationSpan[]): WaterfallData {
  return { spans, total_duration_ms: 1_000, span_count: spans.length }
}

const byName = (span: VisualizationSpan) => span.name

describe('deriveSpanGroups', () => {
  it('ranks the busiest groups first, ties alphabetical', () => {
    const groups = deriveSpanGroups(
      [
        vis({ span_id: 'a1', name: 'chat.respond' }),
        vis({ span_id: 'n1', name: 'session::update-message' }),
        vis({ span_id: 'n2', name: 'session::update-message' }),
        vis({ span_id: 'n3', name: 'session::update-message' }),
        vis({ span_id: 'b1', name: 'auth.verify' }),
      ],
      byName,
    )
    expect(groups).toEqual([
      { key: 'session::update-message', count: 3 },
      { key: 'auth.verify', count: 1 },
      { key: 'chat.respond', count: 1 },
    ])
  })

  it('skips spans whose key resolves to null', () => {
    const groups = deriveSpanGroups(
      [vis({ span_id: 'a' }), vis({ span_id: 'b' })],
      () => null,
    )
    expect(groups).toEqual([])
  })
})

function selection(overrides: {
  hiddenGroups?: string[]
  hiddenWorkers?: string[]
  shownInternal?: string[]
}) {
  return {
    hiddenGroups: new Set(overrides.hiddenGroups ?? []),
    hiddenWorkers: new Set(overrides.hiddenWorkers ?? []),
    shownInternal: new Set(overrides.shownInternal ?? []),
  }
}

describe('applyHiddenSpanFilters', () => {
  const data = waterfall([
    vis({ span_id: 'root', name: 'root' }),
    vis({ span_id: 'n', name: 'noise', parent_span_id: 'root', depth: 1 }),
    vis({
      span_id: 'n-child',
      name: 'db.write',
      parent_span_id: 'n',
      depth: 2,
    }),
    vis({
      span_id: 'n-grandchild',
      name: 'tcp',
      parent_span_id: 'n-child',
      depth: 3,
    }),
    vis({ span_id: 'kept', name: 'work', parent_span_id: 'root', depth: 1 }),
  ])

  it('hides ONLY the matched span; children re-attach to its parent', () => {
    const out = applyHiddenSpanFilters(
      data,
      byName,
      selection({ hiddenGroups: ['noise'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual([
      'root',
      'n-child',
      'n-grandchild',
      'kept',
    ])
    const child = out.spans.find((s) => s.span_id === 'n-child')
    expect(child?.parent_span_id).toBe('root')
    // Depth follows the rewritten chain, not the original one.
    expect(child?.depth).toBe(1)
    const grandchild = out.spans.find((s) => s.span_id === 'n-grandchild')
    expect(grandchild?.parent_span_id).toBe('n-child')
    expect(grandchild?.depth).toBe(2)
    expect(out.span_count).toBe(4)
    expect(out.total_duration_ms).toBe(data.total_duration_ms)
  })

  it('is an identity when nothing is hidden or nothing matches', () => {
    expect(applyHiddenSpanFilters(data, byName, selection({}))).toBe(data)
    expect(
      applyHiddenSpanFilters(
        data,
        byName,
        selection({ hiddenGroups: ['absent'], hiddenWorkers: ['absent'] }),
      ),
    ).toBe(data)
  })

  it('hides a worker span-by-span, keeping what its calls caused', () => {
    const workers = waterfall([
      vis({ span_id: 'root', name: 'root', service_name: 'gateway' }),
      vis({
        span_id: 'pg',
        name: 'query',
        service_name: 'postgres',
        parent_span_id: 'root',
        depth: 1,
      }),
      vis({
        span_id: 'pg-child',
        name: 'tcp',
        service_name: 'net',
        parent_span_id: 'pg',
        depth: 2,
      }),
      vis({
        span_id: 'kept',
        name: 'work',
        service_name: 'agent',
        parent_span_id: 'root',
        depth: 1,
      }),
    ])
    const out = applyHiddenSpanFilters(
      workers,
      byName,
      selection({ hiddenWorkers: ['postgres'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual([
      'root',
      'pg-child',
      'kept',
    ])
    expect(
      out.spans.find((s) => s.span_id === 'pg-child')?.parent_span_id,
    ).toBe('root')
  })

  it('combines hidden groups and hidden workers in one pass', () => {
    const out = applyHiddenSpanFilters(
      data,
      byName,
      selection({ hiddenGroups: ['work'], hiddenWorkers: ['noise'] }),
    )
    // `noise` matches via the worker fallback (name prefix), `work` via its
    // group; the noise span's children stay, promoted to the root.
    expect(out.spans.map((s) => s.span_id)).toEqual([
      'root',
      'n-child',
      'n-grandchild',
    ])
  })

  it('survives malformed parent cycles without self-parenting', () => {
    const cyclic = waterfall([
      vis({ span_id: 'a', name: 'noise', parent_span_id: 'b' }),
      vis({ span_id: 'b', name: 'other', parent_span_id: 'a' }),
    ])
    const out = applyHiddenSpanFilters(
      cyclic,
      byName,
      selection({ hiddenGroups: ['noise'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual(['b'])
    // b's parent (a) is hidden and a's parent is b itself — the walk must
    // not produce a self-parented span.
    expect(out.spans[0].parent_span_id).toBeUndefined()
  })
})

describe('workerGroupKey', () => {
  it('groups by service_name, falling back to the name prefix', () => {
    expect(workerGroupKey(vis({ service_name: 'agent' }))).toBe('agent')
    expect(workerGroupKey(vis({ name: 'billing.charge' }))).toBe('billing')
  })
})

describe('traceSpanGroupKey (page grouping)', () => {
  it('groups by the owning function id, so one entry covers a call family', () => {
    expect(
      traceSpanGroupKey(
        vis({
          name: 'call session::update-message',
          attributes: { function_id: 'session::update-message' },
        }),
      ),
    ).toBe('session::update-message')
    expect(
      traceSpanGroupKey(
        vis({
          name: 'HTTP POST',
          attributes: { 'iii.function.id': 'session::update-message' },
        }),
      ),
    ).toBe('session::update-message')
  })

  it('falls back to the operation name when nothing names a function', () => {
    expect(traceSpanGroupKey(vis({ name: 'GET /health' }))).toBe('GET /health')
  })

  it('groups a tag ROOT under its own name, echoes under their baggage', () => {
    const execute = vis({
      span_id: 'exec',
      name: 'execute harness::turn',
      attributes: { 'iii.function.id': 'harness::turn' },
    })
    const step = vis({
      span_id: 'step',
      name: 'harness::turn step',
      parent_span_id: 'exec',
      attributes: {
        'iii.function.id': 'harness::turn',
        'iii.tag.kind': 'harness.turn',
      },
    })
    // The baggage smear: a child repeating the scope's kind is an echo,
    // not a new segment — it keeps its own function attribution.
    const echo = vis({
      span_id: 'echo',
      name: 'execute session::append',
      parent_span_id: 'step',
      attributes: {
        'iii.function.id': 'session::append',
        'iii.tag.kind': 'harness.turn',
      },
    })
    const spansById = new Map([execute, step, echo].map((s) => [s.span_id, s]))
    expect(traceSpanGroupKey(execute, spansById)).toBe('harness::turn')
    expect(traceSpanGroupKey(step, spansById)).toBe('harness::turn step')
    expect(traceSpanGroupKey(echo, spansById)).toBe('session::append')
    // Without the trace context the tag-root rule is unavailable and the
    // key degrades to the baggage attribution.
    expect(traceSpanGroupKey(step)).toBe('harness::turn')
  })
})

describe('applyHiddenSpanFilters + the harness::turn shape', () => {
  // The real dispatch chain: enqueue + fn_queue + execute are machinery of
  // `harness::turn`; the `harness::turn step` scope span (a tag root, its
  // own group) is the turn itself, with the actual work nested under it.
  const turn = waterfall([
    vis({ span_id: 'send', name: 'execute harness::send' }),
    vis({
      span_id: 'enqueue',
      name: 'enqueue harness::turn → default',
      parent_span_id: 'send',
      depth: 1,
      attributes: { function_id: 'harness::turn' },
    }),
    vis({
      span_id: 'fnq',
      name: 'fn_queue default',
      parent_span_id: 'send',
      depth: 1,
      attributes: {
        function_id: 'harness::turn',
        'iii.tag.kind': 'queue.process',
      },
    }),
    vis({
      span_id: 'exec',
      name: 'execute harness::turn',
      parent_span_id: 'fnq',
      depth: 2,
      attributes: { 'iii.function.id': 'harness::turn' },
    }),
    vis({
      span_id: 'step',
      name: 'harness::turn step',
      parent_span_id: 'exec',
      depth: 3,
      attributes: {
        'iii.function.id': 'harness::turn',
        'iii.tag.kind': 'harness.turn',
      },
    }),
    vis({
      span_id: 'llm',
      name: 'execute router::chat',
      parent_span_id: 'step',
      depth: 4,
      attributes: {
        'iii.function.id': 'router::chat',
        'iii.tag.kind': 'harness.turn',
      },
    }),
    vis({
      span_id: 'append',
      name: 'execute session::append',
      parent_span_id: 'step',
      depth: 4,
      attributes: {
        'iii.function.id': 'session::append',
        'iii.tag.kind': 'harness.turn',
      },
    }),
  ])
  const keyOf = traceSpanGroupKey

  it('hides the dispatch machinery; the step re-parents under the sender', () => {
    const out = applyHiddenSpanFilters(
      turn,
      keyOf,
      selection({ hiddenGroups: ['harness::turn'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual([
      'send',
      'step',
      'llm',
      'append',
    ])
    const step = out.spans.find((s) => s.span_id === 'step')
    expect(step?.parent_span_id).toBe('send')
    expect(step?.depth).toBe(1)
    expect(out.spans.find((s) => s.span_id === 'llm')?.depth).toBe(2)
  })

  it('spans matching on their own hide even when their parent survives', () => {
    const out = applyHiddenSpanFilters(
      turn,
      keyOf,
      selection({ hiddenGroups: ['harness::turn', 'session::append'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual(['send', 'step', 'llm'])
  })

  it('hiding the step group keeps its children, promoted to the executor', () => {
    const out = applyHiddenSpanFilters(
      turn,
      keyOf,
      selection({ hiddenGroups: ['harness::turn step'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual([
      'send',
      'enqueue',
      'fnq',
      'exec',
      'llm',
      'append',
    ])
    expect(out.spans.find((s) => s.span_id === 'llm')?.parent_span_id).toBe(
      'exec',
    )
  })

  it('hides internal-tagged spans by default, reveals per family', () => {
    // Call-site tagging (`iii.tag.hidden = <family>`): the harness's state
    // bookkeeping and session-manager's event fan-out ride a baggage scope
    // that stamps the tag on the whole delivery subtree. Hidden by default
    // — no funnel interaction required — and revealed per family.
    const data = waterfall([
      vis({ span_id: 'step', name: 'harness::turn step' }),
      vis({
        span_id: 'st-1',
        name: 'call state::get',
        parent_span_id: 'step',
        attributes: {
          function_id: 'state::get',
          'iii.tag.hidden': 'harness state',
        },
      }),
      vis({
        span_id: 'ev-1',
        name: 'call iii::console::session_live::abc',
        parent_span_id: 'step',
        attributes: { 'iii.tag.hidden': 'session events' },
      }),
      vis({
        span_id: 'work',
        name: 'execute router::chat',
        parent_span_id: 'step',
      }),
    ])

    const hiddenByDefault = applyHiddenSpanFilters(data, keyOf, selection({}))
    expect(hiddenByDefault.spans.map((s) => s.span_id)).toEqual([
      'step',
      'work',
    ])

    const stateShown = applyHiddenSpanFilters(
      data,
      keyOf,
      selection({ shownInternal: ['harness state'] }),
    )
    expect(stateShown.spans.map((s) => s.span_id)).toEqual([
      'step',
      'st-1',
      'work',
    ])
  })

  it('an untagged child of a hidden internal span survives, promoted', () => {
    // Only the tagged spans themselves hide: if a hidden internal call has
    // an untagged descendant (a downstream worker on an SDK that drops the
    // baggage), the descendant stays visible under the hidden span's parent.
    const data = waterfall([
      vis({ span_id: 'step', name: 'harness::turn step' }),
      vis({
        span_id: 'ev-1',
        name: 'call iii::console::session_live::abc',
        parent_span_id: 'step',
        depth: 1,
        attributes: { 'iii.tag.hidden': 'session events' },
      }),
      vis({
        span_id: 'relay',
        name: 'execute console::relay',
        parent_span_id: 'ev-1',
        depth: 2,
      }),
    ])
    const out = applyHiddenSpanFilters(data, keyOf, selection({}))
    expect(out.spans.map((s) => s.span_id)).toEqual(['step', 'relay'])
    const relay = out.spans.find((s) => s.span_id === 'relay')
    expect(relay?.parent_span_id).toBe('step')
    expect(relay?.depth).toBe(1)
  })
})

describe('reparentThroughHidden (strip bars)', () => {
  const bars = [
    { id: 'send', parentId: undefined, label: 'harness::send' },
    { id: 'fnq', parentId: 'send', label: 'fn_queue' },
    { id: 'exec', parentId: 'fnq', label: 'harness::turn' },
    { id: 'step', parentId: 'exec', label: 'turn step' },
    { id: 'late', parentId: 'not-arrived', label: 'orphan' },
  ]

  it('drops hidden bars and re-points children to the nearest kept ancestor', () => {
    const kept = reparentThroughHidden(
      bars,
      (b) => b.id !== 'fnq' && b.id !== 'exec',
    )
    expect(kept.map((b) => b.id)).toEqual(['send', 'step', 'late'])
    expect(kept.find((b) => b.id === 'step')?.parentId).toBe('send')
  })

  it('keeps unknown parent ids verbatim (late arrivals stay connectable)', () => {
    const kept = reparentThroughHidden(bars, () => true)
    expect(kept.find((b) => b.id === 'late')?.parentId).toBe('not-arrived')
  })

  it('breaks parent cycles instead of looping or self-parenting', () => {
    const cyclic = [
      { id: 'a', parentId: 'b' },
      { id: 'b', parentId: 'a' },
    ]
    const kept = reparentThroughHidden(cyclic, (b) => b.id === 'b')
    expect(kept.map((b) => b.id)).toEqual(['b'])
    expect(kept[0].parentId).toBeUndefined()
  })
})
