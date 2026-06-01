import { ChevronRight } from 'lucide-react'
import { useMemo, useState } from 'react'
import { cn } from '@/lib/utils'
import { percentOfTotal } from '../lib/percent'
import { getServiceColor } from '../lib/traceColors'
import type { WaterfallData } from '../lib/traceTransform'
import { formatDuration, getServiceName } from '../lib/traceUtils'

interface ServiceBreakdownProps {
  data: WaterfallData
}

interface ServiceStats {
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

export function ServiceBreakdown({ data }: ServiceBreakdownProps) {
  const [isExpanded, setIsExpanded] = useState(true)

  const { serviceStats, percentiles } = useMemo(() => {
    const statsMap = new Map<string, ServiceStats>()
    const durations: number[] = []

    for (const span of data.spans) {
      const serviceName = getServiceName(span)
      if (Number.isFinite(span.duration_ms)) durations.push(span.duration_ms)

      if (!statsMap.has(serviceName)) {
        statsMap.set(serviceName, {
          name: serviceName,
          color: getServiceColor(serviceName),
          spanCount: 0,
          totalDuration: 0,
          errorCount: 0,
          percentage: 0,
        })
      }

      const stats = statsMap.get(serviceName)
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
      // Guarded: zero-duration traces (total_duration_ms === 0) would
      // otherwise yield NaN/Infinity bar widths. Clamped to [0, 100].
      stats.percentage = percentOfTotal(stats.totalDuration, totalDuration)
    }

    const serviceStats = Array.from(statsMap.values()).sort(
      (a, b) => b.totalDuration - a.totalDuration,
    )

    const percentiles = {
      p50: calculatePercentile(durations, 50),
      p95: calculatePercentile(durations, 95),
      p99: calculatePercentile(durations, 99),
      max: durations.length > 0 ? Math.max(...durations) : 0,
    }

    return { serviceStats, percentiles }
  }, [data])

  return (
    <div className="bg-panel">
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center gap-2 px-4 py-2.5 hover:bg-bg transition-colors text-left"
        aria-label="toggle services section"
        aria-expanded={isExpanded}
      >
        <ChevronRight
          className={cn(
            'w-3 h-3 text-ink-faint transition-transform duration-150',
            isExpanded && 'rotate-90',
          )}
        />
        <span className="font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
          services
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
          {serviceStats.map((service) => (
            <div key={service.name} className="group">
              <div className="flex items-center gap-2 mb-1">
                <span
                  aria-hidden
                  className="w-2 h-2 flex-shrink-0"
                  style={{ backgroundColor: service.color }}
                />
                <span className="font-mono text-[12px] text-ink truncate flex-1 lowercase">
                  {service.name}
                </span>
                <span className="font-mono text-[11px] text-ink-faint tabular-nums">
                  {service.spanCount} span{service.spanCount !== 1 ? 's' : ''}
                </span>
                <span className="font-mono text-[11px] text-ink tabular-nums w-16 text-right">
                  {formatDuration(service.totalDuration)}
                </span>
                {service.errorCount > 0 && (
                  <span className="font-mono text-[11px] text-alert font-semibold tabular-nums">
                    {service.errorCount} err
                  </span>
                )}
              </div>
              <div className="h-1 bg-bg border border-rule overflow-hidden ml-4">
                <div
                  className="h-full transition-all duration-300"
                  style={{
                    width: `${Math.max(2, service.percentage)}%`,
                    backgroundColor: service.color,
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
