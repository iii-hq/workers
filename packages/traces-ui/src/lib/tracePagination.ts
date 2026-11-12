import type { TraceListItem } from '../hooks/useTraceData'

export interface TracePageStats {
  /** Total traces matching the engine-side filters, before pagination. */
  totalTraces: number
  /** Rows from the current server page that remain visible locally. */
  pageTraceCount: number
  /** Error rows on the current visible page, never presented as global. */
  errorCount: number
  /** Average duration of the current visible page. */
  avgDuration: number
}

export function tracePageCount(totalTraces: number, pageSize: number): number {
  return Math.max(1, Math.ceil(totalTraces / pageSize))
}

export function buildTracePageStats(
  rows: readonly TraceListItem[],
  totalTraces: number,
): TracePageStats {
  return {
    totalTraces,
    pageTraceCount: rows.length,
    errorCount: rows.filter((trace) => trace.status === 'error').length,
    avgDuration:
      rows.length > 0
        ? rows.reduce((sum, trace) => sum + (trace.duration ?? 0), 0) /
          rows.length
        : 0,
  }
}
