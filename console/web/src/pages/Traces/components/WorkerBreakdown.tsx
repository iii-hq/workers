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

export function WorkerBreakdown({ data }: WorkerBreakdownProps) {
  const [isExpanded, setIsExpanded] = useState(true)

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
        className="w-full flex items-center gap-2 px-4 py-2.5 hover:bg-bg transition-colors text-left"
        aria-label="toggle workers section"
        aria-expanded={isExpanded}
      >
        <ChevronRight
          className={cn(
            'w-3 h-3 text-ink-faint transition-transform duration-150',
            isExpanded && 'rotate-90',
          )}
        />
        <span className="font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
          workers
        </span>

        <div className="flex items-center gap-3 ml-auto font-mono text-[11px] text-ink-faint tabular-nums">
          <span>
            p50{' '}
            <span className="text-ink">{formatDuration(percentiles.p50)}</span>
          </span>
          <span>
            p95{' '}
            <span className="text-ink">{formatDuration(percentiles.p95)}</span>
          </span>
          <span>
            p99{' '}
            <span className="text-ink">{formatDuration(percentiles.p99)}</span>
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
                  className="w-2 h-2 flex-shrink-0"
                  style={{ backgroundColor: worker.color }}
                />
                <span className="font-mono text-[12px] text-ink truncate flex-1 lowercase">
                  {worker.name}
                </span>
                <span className="font-mono text-[11px] text-ink-faint tabular-nums">
                  {worker.spanCount} span{worker.spanCount !== 1 ? 's' : ''}
                </span>
                <span className="font-mono text-[11px] text-ink tabular-nums w-16 text-right">
                  {formatDuration(worker.totalDuration)}
                </span>
                {worker.errorCount > 0 && (
                  <span className="font-mono text-[11px] text-alert font-semibold tabular-nums">
                    {worker.errorCount} err
                  </span>
                )}
              </div>
              <div className="h-1 bg-bg border border-rule overflow-hidden ml-4">
                <div
                  className="h-full transition-all duration-300"
                  style={{
                    width: `${Math.max(2, worker.percentage)}%`,
                    backgroundColor: worker.color,
                  }}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
