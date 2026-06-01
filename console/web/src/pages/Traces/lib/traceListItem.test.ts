// Tests for the pure span -> view-model mapper extracted from
// `useTraceData`. Covers attribute-shape normalization (array vs
// object), status normalization, OTel semantic attribute lookup, and
// fingerprinting for fetch-result dedup.

import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import {
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

describe('mapSpanToListItem — non-string status (unvalidated API boundary)', () => {
  // The engine declares status as a string, but some OTel encoders emit it
  // as a numeric code. A non-string value must not crash the whole list
  // render (the mapper runs inside `spans.map` under the page ErrorBoundary).
  it('maps the numeric OTel error code (2) to status="error"', () => {
    const span = makeSpan({ status: 2 as unknown as string })
    expect(mapSpanToListItem(span).status).toBe('error')
  })

  it.each([1, 0])('maps numeric non-error code %s to status="ok"', (code) => {
    const span = makeSpan({ status: code as unknown as string })
    expect(mapSpanToListItem(span).status).toBe('ok')
  })

  it.each([
    ['null', null],
    ['undefined', undefined],
    ['object', { code: 2 }],
  ])('does not throw and defaults to "ok" for %s status', (_label, raw) => {
    const span = makeSpan({ status: raw as unknown as string })
    expect(() => mapSpanToListItem(span)).not.toThrow()
    expect(mapSpanToListItem(span).status).toBe('ok')
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
