import { AlertCircle, ChevronRight, Clock, Copy, Layers, X } from 'lucide-react'
import { Fragment, useMemo } from 'react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import { getWorkerColor } from '../lib/traceColors'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import {
  formatDuration,
  getWorkerName,
  useCopyToClipboard,
} from '../lib/traceUtils'

interface TraceHeaderProps {
  data: WaterfallData
  traceId: string
  onClose: () => void
  onSpanClick?: (span: VisualizationSpan) => void
}

export function TraceHeader({
  data,
  traceId,
  onClose,
  onSpanClick,
}: TraceHeaderProps) {
  const { copiedKey, copy } = useCopyToClipboard()
  const copied = copiedKey === 'traceId'
  const rootSpan = data.spans.find((s) => s.depth === 0)

  const criticalPath = useMemo(() => {
    if (!data.spans.length) return []

    const childrenMap = new Map<string, VisualizationSpan[]>()
    const rootSpans: VisualizationSpan[] = []

    for (const span of data.spans) {
      if (!span.parent_span_id) {
        rootSpans.push(span)
      } else {
        const children = childrenMap.get(span.parent_span_id) || []
        children.push(span)
        childrenMap.set(span.parent_span_id, children)
      }
    }

    const path: VisualizationSpan[] = []
    let current = rootSpans.sort((a, b) => b.duration_ms - a.duration_ms)[0]

    while (current) {
      path.push(current)
      const children = childrenMap.get(current.span_id) || []
      current = children.sort((a, b) => b.duration_ms - a.duration_ms)[0]
    }

    return path
  }, [data.spans])

  const { errorCount, workerList, workerDurations } = useMemo(() => {
    let errorCount = 0
    const workers = new Set<string>()
    const durationMap = new Map<string, number>()

    for (const span of data.spans) {
      if (span.status === 'error') errorCount++
      const worker = getWorkerName(span)
      if (span.service_name) workers.add(span.service_name)
      durationMap.set(worker, (durationMap.get(worker) || 0) + span.duration_ms)
    }

    const workerList = Array.from(workers).filter((s): s is string =>
      Boolean(s),
    )
    return { errorCount, workerList, workerDurations: durationMap }
  }, [data])

  const hasErrors = errorCount > 0
  const workerCount = Math.max(workerList.length, 1)
  const rootWorker = rootSpan ? getWorkerName(rootSpan) : 'trace'

  return (
    <div className="bg-panel border-b border-rule-2 flex-shrink-0">
      <div className="flex items-center gap-2 px-4 pt-3 pb-1.5">
        <span className="px-1.5 py-0.5 font-mono text-[11px] font-medium uppercase tracking-[0.06em] flex-shrink-0 border border-rule bg-bg text-ink-faint lowercase">
          {rootWorker}
        </span>
        <h2
          className="font-mono text-[13px] font-semibold text-ink leading-tight truncate flex-1 min-w-0 lowercase"
          title={rootSpan?.name || 'trace details'}
        >
          {rootSpan?.name || 'trace details'}
        </h2>
        <Button
          variant="icon"
          size="icon"
          onClick={onClose}
          aria-label="close trace detail"
          title="close (esc)"
        >
          <X className="w-3.5 h-3.5" />
        </Button>
      </div>

      <div className="flex items-center gap-2 px-4 pb-2.5 flex-wrap">
        <button
          type="button"
          onClick={() => copy('traceId', traceId)}
          className="flex items-center gap-1 font-mono text-[11px] text-ink-faint hover:text-ink transition-colors group"
        >
          <span className="tabular-nums">{traceId.substring(0, 12)}</span>
          {copied ? (
            <span className="text-accent text-[10px] uppercase tracking-[0.06em]">
              copied
            </span>
          ) : (
            <Copy className="w-2.5 h-2.5 opacity-0 group-hover:opacity-100 transition-opacity" />
          )}
        </button>

        <span aria-hidden className="w-px h-3 bg-rule-2" />

        <span className="flex items-center gap-1 px-2 py-0.5 bg-bg border border-rule">
          <Clock className="w-2.5 h-2.5 text-accent" />
          <span className="text-[11px] font-mono font-semibold text-accent tabular-nums">
            {formatDuration(data.total_duration_ms)}
          </span>
        </span>

        <span className="flex items-center gap-1 px-2 py-0.5 bg-bg border border-rule">
          <Layers className="w-2.5 h-2.5 text-ink-faint" />
          <span className="text-[11px] font-mono text-ink-faint tabular-nums">
            {data.span_count} spans
          </span>
        </span>

        <span className="flex items-center gap-1 px-2 py-0.5 bg-bg border border-rule">
          <span className="text-[11px] font-mono text-ink-faint tabular-nums">
            {workerCount} worker{workerCount === 1 ? '' : 's'}
          </span>
        </span>

        {hasErrors && (
          <span className="flex items-center gap-1 px-2 py-0.5 bg-bg border border-alert">
            <AlertCircle className="w-2.5 h-2.5 text-alert" />
            <span className="text-[11px] font-mono font-semibold text-alert tabular-nums">
              {errorCount} err
            </span>
          </span>
        )}
      </div>

      {workerList.length > 1 && (
        <div className="px-4 pb-2.5">
          <div className="flex h-1.5 bg-bg border border-rule overflow-hidden">
            {workerList.map((worker, i) => {
              const workerDuration = workerDurations.get(worker) || 0
              const pct = (workerDuration / data.total_duration_ms) * 100
              return (
                <div
                  key={worker}
                  className={cn(
                    'h-full transition-all duration-300',
                    i > 0 && 'border-l border-rule-2',
                  )}
                  style={{
                    width: `${Math.max(2, pct)}%`,
                    backgroundColor: getWorkerColor(worker),
                  }}
                  title={`${worker}: ${pct.toFixed(1)}%`}
                />
              )
            })}
          </div>
          <div className="flex items-center gap-3 mt-1.5 flex-wrap">
            {workerList.map((worker) => (
              <div
                key={worker}
                className="flex items-center gap-1 font-mono text-[10px] text-ink-faint"
              >
                <span
                  aria-hidden
                  className="w-1.5 h-1.5 flex-shrink-0"
                  style={{ backgroundColor: getWorkerColor(worker) }}
                />
                <span className="truncate lowercase">{worker}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {criticalPath.length > 1 && (
        <div className="flex items-center gap-1 px-4 pb-2.5 overflow-x-auto">
          {criticalPath.map((span, i) => (
            <Fragment key={span.span_id}>
              {i > 0 && (
                <ChevronRight className="w-3 h-3 text-ink-ghost flex-shrink-0" />
              )}
              <button
                type="button"
                onClick={() => onSpanClick?.(span)}
                className="font-mono text-[11px] text-ink-faint hover:text-ink hover:bg-bg truncate max-w-[140px] flex-shrink-0 px-1 py-0.5 transition-colors lowercase"
                title={span.name}
              >
                {span.name}
              </button>
            </Fragment>
          ))}
        </div>
      )}
    </div>
  )
}
