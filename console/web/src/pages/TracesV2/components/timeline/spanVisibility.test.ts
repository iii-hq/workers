import { describe, expect, it } from 'vitest'
import { traceSpanGroupKey } from '../../lib/traceTimelineFilters'
import type { VisualizationSpan, WaterfallData } from '../../lib/traceTransform'
import {
  applyHiddenSpanFilters,
  deriveSpanGroups,
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
}) {
  return {
    hiddenGroups: new Set(overrides.hiddenGroups ?? []),
    hiddenWorkers: new Set(overrides.hiddenWorkers ?? []),
  }
}

describe('applyHiddenSpanFilters', () => {
  const data = waterfall([
    vis({ span_id: 'root', name: 'root' }),
    vis({ span_id: 'n', name: 'noise', parent_span_id: 'root' }),
    vis({ span_id: 'n-child', name: 'db.write', parent_span_id: 'n' }),
    vis({ span_id: 'n-grandchild', name: 'tcp', parent_span_id: 'n-child' }),
    vis({ span_id: 'kept', name: 'work', parent_span_id: 'root' }),
  ])

  it('hides a group together with its whole subtrees, keeping the window', () => {
    const out = applyHiddenSpanFilters(
      data,
      byName,
      selection({ hiddenGroups: ['noise'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual(['root', 'kept'])
    expect(out.span_count).toBe(2)
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

  it('hides a whole worker (service_name) with its subtrees', () => {
    const workers = waterfall([
      vis({ span_id: 'root', name: 'root', service_name: 'gateway' }),
      vis({
        span_id: 'pg',
        name: 'query',
        service_name: 'postgres',
        parent_span_id: 'root',
      }),
      vis({
        span_id: 'pg-child',
        name: 'tcp',
        service_name: 'net',
        parent_span_id: 'pg',
      }),
      vis({
        span_id: 'kept',
        name: 'work',
        service_name: 'agent',
        parent_span_id: 'root',
      }),
    ])
    const out = applyHiddenSpanFilters(
      workers,
      byName,
      selection({ hiddenWorkers: ['postgres'] }),
    )
    expect(out.spans.map((s) => s.span_id)).toEqual(['root', 'kept'])
  })

  it('combines hidden groups and hidden workers in one pass', () => {
    const out = applyHiddenSpanFilters(
      data,
      byName,
      selection({ hiddenGroups: ['work'], hiddenWorkers: ['noise'] }),
    )
    // `noise` spans match via the worker fallback (name prefix), `work`
    // via its group — only the root survives.
    expect(out.spans.map((s) => s.span_id)).toEqual(['root'])
  })

  it('survives malformed parent cycles', () => {
    const cyclic = waterfall([
      vis({ span_id: 'a', name: 'noise', parent_span_id: 'b' }),
      vis({ span_id: 'b', name: 'other', parent_span_id: 'a' }),
    ])
    const out = applyHiddenSpanFilters(
      cyclic,
      byName,
      selection({ hiddenGroups: ['noise'] }),
    )
    expect(out.spans).toEqual([])
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
})
