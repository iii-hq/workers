// Tests for the pure span -> view-model mapper extracted from
// `useTraceData`. Covers attribute-shape normalization (array vs
// object), status normalization, OTel semantic attribute lookup, and
// fingerprinting for fetch-result dedup.

import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import {
  dedupeToTraceRoots,
  fingerprintTraceList,
  mapSpanToListItem,
  normalizeSpanAttributes,
} from './traceListItem'

const NS_PER_MS = 1_000_000

function makeSpan(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return {
    trace_id: 't-1',
    span_id: 's-1',
    name: 'GET /api/users',
    start_time_unix_nano: 1_700_000_000_000 * NS_PER_MS,
    end_time_unix_nano: (1_700_000_000_000 + 42) * NS_PER_MS,
    status: 'OK',
    attributes: [],
    events: [],
    links: [],
    service_name: 'api-gateway',
    ...overrides,
  }
}

describe('dedupeToTraceRoots', () => {
  it('collapses a search_all_spans flood to one root per trace', () => {
    // What the engine returns for a `harness::send` search: the whole turn.
    const spans = [
      makeSpan({
        span_id: 'root',
        name: 'handle_invocation harness::send',
        start_time_unix_nano: 100,
      }),
      makeSpan({
        span_id: 'call',
        parent_span_id: 'root',
        name: 'call harness::send',
        start_time_unix_nano: 110,
      }),
      makeSpan({
        span_id: 'child',
        parent_span_id: 'call',
        name: 'harness::turn',
        start_time_unix_nano: 120,
      }),
    ]

    const out = dedupeToTraceRoots(spans)
    expect(out).toHaveLength(1)
    expect(out[0].span_id).toBe('root')
  })

  it('keeps one root per distinct trace', () => {
    const out = dedupeToTraceRoots([
      makeSpan({ trace_id: 'a', span_id: 'a-root' }),
      makeSpan({ trace_id: 'a', span_id: 'a-child', parent_span_id: 'a-root' }),
      makeSpan({ trace_id: 'b', span_id: 'b-root' }),
    ])
    expect(out.map((s) => s.trace_id).sort()).toEqual(['a', 'b'])
    expect(out.find((s) => s.trace_id === 'a')?.span_id).toBe('a-root')
  })

  it('is a no-op for a roots-only list (non-search response)', () => {
    const spans = [
      makeSpan({ trace_id: 'a', span_id: 'a' }),
      makeSpan({ trace_id: 'b', span_id: 'b' }),
    ]
    expect(dedupeToTraceRoots(spans)).toHaveLength(2)
  })

  it('falls back to the earliest span when no root is present', () => {
    // Root span aged out of the ring buffer; keep the earliest survivor.
    const out = dedupeToTraceRoots([
      makeSpan({
        span_id: 'late',
        parent_span_id: 'gone',
        start_time_unix_nano: 200,
      }),
      makeSpan({
        span_id: 'early',
        parent_span_id: 'gone',
        start_time_unix_nano: 150,
      }),
    ])
    expect(out).toHaveLength(1)
    expect(out[0].span_id).toBe('early')
  })
})

describe('normalizeSpanAttributes', () => {
  it('normalizes the array-of-tuples shape into a flat object', () => {
    const out = normalizeSpanAttributes([
      ['http.method', 'GET'],
      ['http.status_code', 200],
      ['user.id', 'u-7'],
    ])
    expect(out).toEqual({
      'http.method': 'GET',
      'http.status_code': 200,
      'user.id': 'u-7',
    })
  })

  it('normalizes the plain-object shape', () => {
    const out = normalizeSpanAttributes({ k1: 'v1', k2: 2 })
    expect(out).toEqual({ k1: 'v1', k2: 2 })
  })

  it('returns an empty object for undefined / null / empty array', () => {
    expect(normalizeSpanAttributes(undefined)).toEqual({})
    expect(normalizeSpanAttributes(null as unknown as undefined)).toEqual({})
    expect(normalizeSpanAttributes([])).toEqual({})
  })

  it('skips malformed tuples (length < 2)', () => {
    const out = normalizeSpanAttributes([
      ['ok', 'value'],
      ['malformed'] as unknown as [string, unknown],
    ])
    expect(out).toEqual({ ok: 'value' })
  })

  it('coerces non-string keys to strings', () => {
    const out = normalizeSpanAttributes([[42 as unknown as string, 'value']])
    expect(out['42']).toBe('value')
  })

  it('returns a fresh object (does not mutate input)', () => {
    const input = { k: 'v' }
    const out = normalizeSpanAttributes(input)
    out.k = 'mutated'
    expect(input.k).toBe('v')
  })
})

describe('mapSpanToListItem — basic shape', () => {
  it('converts unix-nano times to ms and computes duration', () => {
    // `toMs` auto-detects ns vs ms by magnitude — values below the
    // ~year-2100-in-ms threshold pass through as-is. Use realistic
    // 2024-era ns timestamps so the divide-by-1M branch fires.
    const startMs = 1_700_000_000_000
    const endMs = 1_700_000_000_500
    const span = makeSpan({
      start_time_unix_nano: startMs * NS_PER_MS,
      end_time_unix_nano: endMs * NS_PER_MS,
    })
    const out = mapSpanToListItem(span)
    expect(out.startTime).toBe(startMs)
    expect(out.endTime).toBe(endMs)
    expect(out.duration).toBe(500)
  })

  it('forwards trace_id, name, and service_name', () => {
    const out = mapSpanToListItem(
      makeSpan({ trace_id: 'abc', name: 'POST /x', service_name: 'svc-x' }),
    )
    expect(out.traceId).toBe('abc')
    expect(out.rootOperation).toBe('POST /x')
    expect(out.services).toEqual(['svc-x'])
  })

  it('falls back to "unknown" when service_name is missing', () => {
    expect(
      mapSpanToListItem(makeSpan({ service_name: undefined })).services,
    ).toEqual(['unknown'])
  })

  it('always emits spanCount = 1', () => {
    expect(mapSpanToListItem(makeSpan()).spanCount).toBe(1)
  })
})

describe('mapSpanToListItem — status normalization', () => {
  it.each([
    'error',
    'Error',
    'ERROR',
    'errOr',
  ])('maps case variant %s of "error" to status="error"', (raw) => {
    expect(mapSpanToListItem(makeSpan({ status: raw })).status).toBe('error')
  })

  it.each([
    'OK',
    'ok',
    'unset',
    'UNSET',
    '',
    'something_else',
  ])('maps non-error status %s to status="ok"', (raw) => {
    expect(mapSpanToListItem(makeSpan({ status: raw })).status).toBe('ok')
  })
})

describe('mapSpanToListItem — semantic attributes', () => {
  it('reads functionId from OTel faas.invoked_name', () => {
    const out = mapSpanToListItem(
      makeSpan({ attributes: [['faas.invoked_name', 'fn-billing']] }),
    )
    expect(out.functionId).toBe('fn-billing')
  })

  it('falls back to legacy function_id when faas.invoked_name is absent', () => {
    const out = mapSpanToListItem(
      makeSpan({ attributes: [['function_id', 'fn-legacy']] }),
    )
    expect(out.functionId).toBe('fn-legacy')
  })

  it('prefers faas.invoked_name over function_id when both are set', () => {
    const out = mapSpanToListItem(
      makeSpan({
        attributes: [
          ['faas.invoked_name', 'fn-new'],
          ['function_id', 'fn-old'],
        ],
      }),
    )
    expect(out.functionId).toBe('fn-new')
  })

  it('reads topic from OTel messaging.destination.name', () => {
    const out = mapSpanToListItem(
      makeSpan({
        attributes: [['messaging.destination.name', 'orders.created']],
      }),
    )
    expect(out.topic).toBe('orders.created')
  })

  it('leaves functionId/topic undefined when neither attribute is present', () => {
    const out = mapSpanToListItem(makeSpan({ attributes: [] }))
    expect(out.functionId).toBeUndefined()
    expect(out.topic).toBeUndefined()
  })

  it('works with the plain-object attributes shape too', () => {
    const out = mapSpanToListItem(
      makeSpan({
        attributes: {
          'faas.invoked_name': 'fn-from-object',
        } as unknown as StoredSpan['attributes'],
      }),
    )
    expect(out.functionId).toBe('fn-from-object')
  })
})

describe('fingerprintTraceList', () => {
  function item(traceId: string): TraceListItem {
    return {
      traceId,
      rootOperation: '_',
      status: 'ok',
      startTime: 0,
      spanCount: 1,
      services: [],
    }
  }

  it('returns the same fingerprint for identical lists', () => {
    const a = [item('t-1'), item('t-2'), item('t-3')]
    const b = [item('t-1'), item('t-2'), item('t-3')]
    expect(fingerprintTraceList(a)).toBe(fingerprintTraceList(b))
  })

  it('differs when order changes (catches sort-direction flips)', () => {
    const ascending = [item('t-1'), item('t-2'), item('t-3')]
    const descending = [item('t-3'), item('t-2'), item('t-1')]
    expect(fingerprintTraceList(ascending)).not.toBe(
      fingerprintTraceList(descending),
    )
  })

  it('differs when middle item changes (was the latent bug in the old fingerprint)', () => {
    const before = [item('t-1'), item('t-MIDDLE'), item('t-9')]
    const after = [item('t-1'), item('t-OTHER'), item('t-9')]
    // Old fingerprint sampled only first + last + count — would have
    // returned the same value here. The all-IDs fingerprint catches it.
    expect(fingerprintTraceList(before)).not.toBe(fingerprintTraceList(after))
  })

  it('returns a distinct fingerprint for an empty list', () => {
    expect(fingerprintTraceList([])).toBe('0:')
  })
})
