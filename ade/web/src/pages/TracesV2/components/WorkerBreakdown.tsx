/**
 * The trace card's footer: how the run's time splits across workers.
 *
 * Collapsed, it is one line — the toggle, the worker count and the latency
 * percentiles. Expanded, one row per worker in a quiet table: a colour
 * mark and the name, span and error counts, the total time, and a share
 * bar against the busiest worker (the percent is its share of all span
 * time). Rows sort busiest first so the worker that owned the run is
 * always on top.
 */

import { ChevronRight } from 'lucide-react'
import { useMemo, useState } from 'react'
import { cn } from '@/lib/utils'
import { getWorkerColor } from '../lib/traceColors'
import type { WaterfallData } from '../lib/traceTransform'
import { formatDuration, getWorkerName } from '../lib/traceUtils'

interface WorkerBreakdownProps {
  data: WaterfallData
}

interface WorkerStats {
  name: string
  color: string
  spanCount: number
  totalDuration: number
  errorCount: number
  percentage: number
}

function calculatePercentile(values: number[], percentile: number): number {
  if (values.length === 0) return 0
  const sorted = [...values].sort((a, b) => a - b)
  const index = Math.ceil((percentile / 100) * sorted.length) - 1
  return sorted[Math.max(0, index)]
}

function Percentile({ label, value }: { label: string; value: number }) {
  return (
    <span className="inline-flex items-baseline gap-1 whitespace-nowrap">
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <span className="font-mono text-[11px] text-foreground tabular-nums">
        {formatDuration(value)}
      </span>
    </span>
  )
}

export function WorkerBreakdown({ data }: WorkerBreakdownProps) {
  // Collapsed by default — the footer stays a one-line summary until the
  // user opts into the per-worker rows.
  const [isExpanded, setIsExpanded] = useState(false)

  const { workerStats, percentiles } = useMemo(() => {
    const statsMap = new Map<string, WorkerStats>()
    const durations: number[] = []

    for (const span of data.spans) {
      const workerName = getWorkerName(span)
      if (Number.isFinite(span.duration_ms)) durations.push(span.duration_ms)

      if (!statsMap.has(workerName)) {
        statsMap.set(workerName, {
          name: workerName,
          color: getWorkerColor(workerName),
          spanCount: 0,
          totalDuration: 0,
          errorCount: 0,
          percentage: 0,
        })
      }

      const stats = statsMap.get(workerName)
      if (stats) {
        stats.spanCount++
        stats.totalDuration += Number.isFinite(span.duration_ms)
          ? span.duration_ms
          : 0
        if (span.status === 'error') stats.errorCount++
      }
    }

    // Share of all span time, not of the trace's wall clock: a parent span
    // runs for the whole of its children, so wall-clock shares sum past
    // 100% and the busiest worker read as "191%". Span time is additive.
    let spanTime = 0
    for (const stats of statsMap.values()) spanTime += stats.totalDuration
    for (const stats of statsMap.values()) {
      stats.percentage =
        spanTime > 0 ? (stats.totalDuration / spanTime) * 100 : 0
    }

    const workerStats = Array.from(statsMap.values()).sort(
      (a, b) => b.totalDuration - a.totalDuration,
    )

    const percentiles = {
      p50: calculatePercentile(durations, 50),
      p95: calculatePercentile(durations, 95),
      p99: calculatePercentile(durations, 99),
      max: durations.length > 0 ? Math.max(...durations) : 0,
    }

    return { workerStats, percentiles }
  }, [data])

  // The share bar is scaled to the busiest worker, so the first row reads
  // as the full width and the rest as fractions of it; the number beside
  // each bar is still the honest share of the trace.
  const maxDuration = workerStats[0]?.totalDuration || 1

  return (
    <div className="@container font-inter" data-worker-breakdown>
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex h-9 w-full items-center gap-2 px-3 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
        aria-label="toggle workers section"
        aria-expanded={isExpanded}
      >
        <ChevronRight
          className={cn(
            'size-4 shrink-0 text-muted-foreground transition-transform duration-[var(--motion-duration-control)]',
            isExpanded && 'rotate-90',
          )}
          aria-hidden
        />
        <span className="text-xs font-semibold text-foreground">Workers</span>
        <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
          {workerStats.length}
        </span>

        <span className="ml-auto flex items-center gap-3">
          <Percentile label="p50" value={percentiles.p50} />
          <Percentile label="p95" value={percentiles.p95} />
          <Percentile label="p99" value={percentiles.p99} />
          <span className="hidden @lg:inline">
            <Percentile label="max" value={percentiles.max} />
          </span>
        </span>
      </button>

      {isExpanded && (
        <div className="border-t border-border">
          <div
            className={cn(
              WORKER_GRID,
              'h-7 items-center gap-3 px-3 text-[11px] text-muted-foreground',
            )}
            aria-hidden
          >
            <span className="pl-4">Worker</span>
            <span className="text-right">Spans</span>
            <span className="text-right">Errors</span>
            <span className="text-right">Time</span>
            <span className="hidden @md:block">Share</span>
            <span className="text-right">%</span>
          </div>
          <ul className="divide-y divide-border border-t border-border">
            {workerStats.map((worker) => (
              <li
                key={worker.name}
                className={cn(
                  WORKER_GRID,
                  'h-8 items-center gap-3 px-3 transition-colors hover:bg-muted/50',
                )}
                data-worker-row={worker.name}
              >
                <span className="flex min-w-0 items-center gap-2">
                  <span
                    aria-hidden
                    className="size-2 shrink-0 rounded-full"
                    style={{ backgroundColor: worker.color }}
                  />
                  <span
                    className="truncate text-xs font-medium text-foreground"
                    title={worker.name}
                  >
                    {worker.name}
                  </span>
                </span>
                <span className="text-right font-mono text-[11px] text-muted-foreground tabular-nums">
                  {worker.spanCount}
                </span>
                <span
                  className={cn(
                    'text-right font-mono text-[11px] tabular-nums',
                    worker.errorCount > 0
                      ? 'font-medium text-alert-strong'
                      : 'text-muted-foreground',
                  )}
                >
                  {worker.errorCount > 0 ? worker.errorCount : '—'}
                </span>
                <span className="text-right font-mono text-[11px] text-foreground tabular-nums">
                  {formatDuration(worker.totalDuration)}
                </span>
                <span
                  className="hidden h-1.5 w-full overflow-hidden rounded-full bg-muted @md:block"
                  role="progressbar"
                  aria-label={`${worker.name} share of span time`}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(
                    Math.min(100, Math.max(0, worker.percentage)),
                  )}
                >
                  <span
                    className="block h-full rounded-full transition-[width] duration-[var(--motion-duration-panel)]"
                    style={{
                      width: `${Math.max(2, Math.min(100, (worker.totalDuration / maxDuration) * 100))}%`,
                      backgroundColor: worker.color,
                    }}
                  />
                </span>
                <span className="text-right font-mono text-[11px] text-muted-foreground tabular-nums">
                  {worker.percentage >= 10
                    ? Math.round(worker.percentage)
                    : worker.percentage.toFixed(1)}
                  %
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

/** name · spans · errors · time · (share bar, from @md) · percent */
const WORKER_GRID =
  'grid grid-cols-[minmax(0,1fr)_3rem_3rem_4.5rem_3rem] @md:grid-cols-[minmax(0,1fr)_3rem_3rem_4.5rem_minmax(0,8rem)_3rem]'
