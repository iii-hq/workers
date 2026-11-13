import { describe, expect, it } from 'vitest'
import type { TraceListItem } from '../hooks/useTraceData'
import { buildTracePageStats, tracePageCount } from './tracePagination'

function trace(
  traceId: string,
  status: TraceListItem['status'],
  duration: number,
): TraceListItem {
  return {
    traceId,
    rootOperation: `execute ${traceId}`,
    status,
    startTime: 0,
    endTime: duration,
    duration,
    spanCount: 1,
    workers: ['worker'],
  }
}

describe('tracePageCount', () => {
  it('uses the engine total instead of the loaded page length', () => {
    expect(tracePageCount(2_601, 50)).toBe(53)
    expect(tracePageCount(0, 50)).toBe(1)
  })
})

describe('buildTracePageStats', () => {
  it('keeps the global total separate from page-scoped errors and average', () => {
    expect(
      buildTracePageStats(
        [trace('ok', 'ok', 10), trace('failed', 'error', 30)],
        2_601,
      ),
    ).toEqual({
      totalTraces: 2_601,
      pageTraceCount: 2,
      errorCount: 1,
      avgDuration: 20,
    })
  })
})
