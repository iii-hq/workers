import { describe, expect, it } from 'vitest'
import {
  collectTraceDetailSpans,
  TRACE_DETAIL_MAX_PAGE_SIZE,
  TRACE_DETAIL_MIN_PAGE_SIZE,
  TRACE_DETAIL_PROBE_PAGE_SIZE,
} from './traceDetailPages'

interface Span {
  span_id: string
  payload?: string
}

function spans(from: number, count: number, payload = ''): Span[] {
  return Array.from({ length: count }, (_, i) => ({
    span_id: `s${from + i}`,
    ...(payload ? { payload } : {}),
  }))
}

/** Serve `all` in windows, recording each requested (offset, limit). */
function pagedFetch(all: Span[], calls: Array<[number, number]>) {
  return (offset: number, limit: number) => {
    calls.push([offset, limit])
    return Promise.resolve({
      spans: all.slice(offset, offset + limit),
      total: all.length,
    })
  }
}

describe('collectTraceDetailSpans', () => {
  it('pages a thin-span trace at the max page size after the probe', async () => {
    const all = spans(0, 700)
    const calls: Array<[number, number]> = []
    const out = await collectTraceDetailSpans(pagedFetch(all, calls))

    expect(out.size).toBe(700)
    expect(calls[0]).toEqual([0, TRACE_DETAIL_PROBE_PAGE_SIZE])
    // Tiny spans price far above the cap — every later page rides the max.
    for (const [, limit] of calls.slice(1)) {
      expect(limit).toBe(TRACE_DETAIL_MAX_PAGE_SIZE)
    }
    // Windows chain without gap or overlap.
    expect(calls.map(([offset]) => offset)).toEqual([0, 50, 300, 550])
  })

  it('prices fat spans down to smaller pages', async () => {
    // ~200KB per span: the 8MiB budget prices ~40 spans per page.
    const all = spans(0, 120, 'x'.repeat(200_000))
    const calls: Array<[number, number]> = []
    const out = await collectTraceDetailSpans(pagedFetch(all, calls))

    expect(out.size).toBe(120)
    const priced = calls[1][1]
    expect(priced).toBeLessThan(TRACE_DETAIL_PROBE_PAGE_SIZE)
    expect(priced).toBeGreaterThanOrEqual(TRACE_DETAIL_MIN_PAGE_SIZE)
  })

  it('halves and retries the same window when a page never arrives', async () => {
    const all = spans(0, 80)
    const calls: Array<[number, number]> = []
    const fetch = (offset: number, limit: number) => {
      calls.push([offset, limit])
      // The first probe "hangs" (an oversized response is never delivered).
      if (calls.length === 1) return new Promise<never>(() => {})
      return Promise.resolve({
        spans: all.slice(offset, offset + limit),
        total: all.length,
      })
    }
    const out = await collectTraceDetailSpans(fetch, 10)

    expect(out.size).toBe(80)
    expect(calls[0]).toEqual([0, TRACE_DETAIL_PROBE_PAGE_SIZE])
    expect(calls[1]).toEqual([0, TRACE_DETAIL_MIN_PAGE_SIZE])
  })

  it('fails once the floor page size cannot be delivered either', async () => {
    await expect(
      collectTraceDetailSpans(() => new Promise<never>(() => {}), 5),
    ).rejects.toThrow(/never arrived/)
  })

  it('propagates a fetch rejection as-is', async () => {
    await expect(
      collectTraceDetailSpans(() => Promise.reject(new Error('boom'))),
    ).rejects.toThrow('boom')
  })

  it('returns empty for a trace with no spans', async () => {
    const calls: Array<[number, number]> = []
    const out = await collectTraceDetailSpans(pagedFetch([], calls))
    expect(out.size).toBe(0)
    expect(calls).toHaveLength(1)
  })

  it('dedupes spans re-served across page boundaries', async () => {
    const all = spans(0, 60)
    let first = true
    const fetch = (offset: number, limit: number) => {
      // Overlap the second window by one span, as a live re-sort might.
      const start = first ? offset : offset - 1
      first = false
      return Promise.resolve({
        spans: all.slice(start, start + limit),
        total: all.length,
      })
    }
    const out = await collectTraceDetailSpans(fetch)
    expect(out.size).toBe(60)
  })
})
