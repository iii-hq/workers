import { describe, expect, it } from 'vitest'
import {
  collectRecentSpanWindow,
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
    const out = await collectTraceDetailSpans(fetch, { timeoutMs: 10 })

    expect(out.size).toBe(80)
    expect(calls[0]).toEqual([0, TRACE_DETAIL_PROBE_PAGE_SIZE])
    expect(calls[1]).toEqual([0, TRACE_DETAIL_MIN_PAGE_SIZE])
  })

  it('fails once the floor page size cannot be delivered either', async () => {
    await expect(
      collectTraceDetailSpans(() => new Promise<never>(() => {}), {
        timeoutMs: 5,
      }),
    ).rejects.toThrow(/never arrived/)
  })

  it('reports each merged page so callers can render progressively', async () => {
    const all = spans(0, 700)
    const calls: Array<[number, number]> = []
    const seen: Array<[number, number]> = []
    const out = await collectTraceDetailSpans(pagedFetch(all, calls), {
      onPage: (accumulated, total) => seen.push([accumulated.size, total]),
    })

    expect(seen).toEqual([
      [50, 700],
      [300, 700],
      [550, 700],
      [700, 700],
    ])
    expect(out.size).toBe(700)
  })

  it('never reports a page for an empty trace', async () => {
    const seen: number[] = []
    await collectTraceDetailSpans(pagedFetch([], []), {
      onPage: (accumulated) => seen.push(accumulated.size),
    })
    expect(seen).toEqual([])
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

describe('collectRecentSpanWindow', () => {
  it('stops at the span budget and reports the true total', async () => {
    const all = spans(0, 2000)
    const calls: Array<[number, number]> = []
    const out = await collectRecentSpanWindow(pagedFetch(all, calls), 250)

    expect(out.spans.length).toBe(250)
    expect(out.total).toBe(2000)
    expect(calls[0]).toEqual([0, TRACE_DETAIL_PROBE_PAGE_SIZE])
    // Windows tile the remainder exactly up to the budget.
    const fetched = calls.reduce((n, [, limit]) => n + limit, 0)
    expect(fetched).toBe(250)
  })

  it('returns the probe alone when it already covers the data', async () => {
    const all = spans(0, 30)
    const calls: Array<[number, number]> = []
    const out = await collectRecentSpanWindow(pagedFetch(all, calls), 250)
    expect(out.spans.length).toBe(30)
    expect(calls).toHaveLength(1)
  })

  it('splits a window in half when its response never arrives', async () => {
    const all = spans(0, 150)
    const calls: Array<[number, number]> = []
    const fetch = (offset: number, limit: number) => {
      calls.push([offset, limit])
      // The first full post-probe window "hangs"; its halves deliver.
      if (offset === 50 && limit === 100) return new Promise<never>(() => {})
      return Promise.resolve({
        spans: all.slice(offset, offset + limit),
        total: all.length,
      })
    }
    // Thin spans price to the max page; force a deterministic split by
    // budgeting exactly one 100-span window after the probe.
    const out = await collectRecentSpanWindow(fetch, 150, 15)
    expect(out.spans.length).toBe(150)
    expect(calls).toContainEqual([50, 50])
    expect(calls).toContainEqual([100, 50])
  })

  it('fails when even the floor window cannot be delivered', async () => {
    await expect(
      collectRecentSpanWindow(() => new Promise<never>(() => {}), 100, 5),
    ).rejects.toThrow(/never arrived/)
  })
})
