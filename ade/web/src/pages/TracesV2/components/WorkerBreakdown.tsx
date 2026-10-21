import { ChevronRight } from 'lucide-react'
import { useMemo, useState } from 'react'
import {
  Progress,
  ProgressIndicator,
  ProgressTrack,
} from '@/components/shadcn/progress'
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

    const totalDuration = data.total_duration_ms
    for (const stats of statsMap.values()) {
      stats.percentage = (stats.totalDuration / totalDuration) * 100
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

  return (
    <div className="bg-panel">
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="flex w-full items-center gap-2 px-4 py-2.5 text-left transition-colors hover:bg-surface-hover"
        aria-label="toggle workers section"
        aria-expanded={isExpanded}
      >
        <ChevronRight
          className={cn(
            'size-4 text-ink-faint transition-transform duration-[var(--motion-duration-control)]',
            isExpanded && 'rotate-90',
          )}
        />
        <span className="text-[13px] font-medium text-ink">Workers</span>

        <div className="ml-auto flex items-center gap-3 text-xs text-ink-faint">
          <span>
            p50{' '}
            <span className="font-mono text-ink tabular-nums">
              {formatDuration(percentiles.p50)}
            </span>
          </span>
          <span>
            p95{' '}
            <span className="font-mono text-ink tabular-nums">
              {formatDuration(percentiles.p95)}
            </span>
          </span>
          <span>
            p99{' '}
            <span className="font-mono text-ink tabular-nums">
              {formatDuration(percentiles.p99)}
            </span>
          </span>
        </div>
      </button>

      {isExpanded && (
        <div className="px-4 pb-3 flex flex-col gap-2.5">
          {workerStats.map((worker) => (
            <div key={worker.name} className="group">
              <div className="flex items-center gap-2 mb-1">
                <span
                  aria-hidden
                  className="size-2 flex-shrink-0 rounded-sm"
                  style={{ backgroundColor: worker.color }}
                />
                <span className="flex-1 truncate text-[13px] font-medium text-ink">
                  {worker.name}
                </span>
                <span className="text-xs text-ink-faint tabular-nums">
                  {worker.spanCount} span{worker.spanCount !== 1 ? 's' : ''}
                </span>
                <span className="w-16 text-right font-mono text-xs text-ink tabular-nums">
                  {formatDuration(worker.totalDuration)}
                </span>
                {worker.errorCount > 0 && (
                  <span className="text-xs font-semibold text-alert tabular-nums">
                    {worker.errorCount} err
                  </span>
                )}
              </div>
              <Progress
                value={Math.max(2, Math.min(100, worker.percentage))}
                aria-label={`${worker.name} share of the trace`}
                className="ml-4"
              >
                <ProgressTrack className="h-1.5">
                  <ProgressIndicator
                    className="rounded-full transition-all duration-[var(--motion-duration-panel)]"
                    style={{ backgroundColor: worker.color }}
                  />
                </ProgressTrack>
              </Progress>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
