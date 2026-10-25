/**
 * The trace card's top: who ran it and what it was, then the numbers that
 * describe the run at a glance — a stats strip of label-over-value cells,
 * the way an observability card reads before its waterfall.
 *
 * Everything shown is derived from the spans already loaded: duration,
 * span and worker counts, the longest span, how many are still running
 * (the Live badge breathes while any are) and how many failed.
 */

import { Check, Copy, LocateFixed, X } from 'lucide-react'
import { useMemo } from 'react'
import { Button } from '@/components/shadcn/button'
import { cn } from '@/lib/utils'
import { formatSpanLabel } from '../lib/spanLabel'
import type { TraceChatLink } from '../lib/traceChatLink'
import { getWorkerColor } from '../lib/traceColors'
import type { VisualizationSpan, WaterfallData } from '../lib/traceTransform'
import {
  formatDuration,
  getWorkerName,
  useCopyToClipboard,
} from '../lib/traceUtils'
import { LiveBadge } from './LiveBadge'

interface TraceHeaderProps {
  data: WaterfallData
  traceId: string
  onClose: () => void
  onSpanClick?: (span: VisualizationSpan) => void
  /** The chat session/turn behind this trace, when its spans carry it. */
  chatLink?: TraceChatLink | null
  /** Open the linked conversation AND land on this trace's turn. */
  onOpenMessage?: (link: TraceChatLink) => void
  /** Non-null while the paged seed still sweeps — the painted detail is
   *  incomplete and the span count counts up instead of stating a total. */
  loadingSpans?: { loaded: number; total: number } | null
}

interface Stat {
  label: string
  value: string
  /** `alert` paints the value red when it is non-zero. */
  tone?: 'alert'
  title?: string
}

function StatCell({ label, value, tone, title }: Stat) {
  const hot = tone === 'alert' && value !== '0'
  return (
    <div className="flex min-w-0 flex-col gap-0.5 px-3 py-2" title={title}>
      <span className="truncate text-[11px] text-muted-foreground">
        {label}
      </span>
      <span
        className={cn(
          'truncate font-mono text-[15px] font-semibold leading-5 tabular-nums',
          hot ? 'text-alert-strong' : 'text-foreground',
        )}
      >
        {value}
      </span>
    </div>
  )
}

export function TraceHeader({
  data,
  traceId,
  onClose,
  chatLink,
  onOpenMessage,
  loadingSpans,
}: TraceHeaderProps) {
  const { copiedKey, copy } = useCopyToClipboard()
  const copied = copiedKey === 'traceId'
  const rootSpan = data.spans.find((s) => s.depth === 0)
  const rootWorker = rootSpan ? getWorkerName(rootSpan) : 'trace'
  const title = rootSpan ? formatSpanLabel(rootSpan) : 'Trace'

  const stats = useMemo(() => {
    let errors = 0
    let running = 0
    let longest: VisualizationSpan | null = null
    const workers = new Set<string>()
    for (const span of data.spans) {
      if (span.status === 'error') errors++
      if (span.pending) running++
      workers.add(getWorkerName(span))
      if (!longest || span.duration_ms > longest.duration_ms) longest = span
    }
    return { errors, running, longest, workers: workers.size }
  }, [data.spans])

  const live = stats.running > 0 || loadingSpans !== null
  const spanCount = loadingSpans
    ? `${Math.min(loadingSpans.loaded, loadingSpans.total)}/${loadingSpans.total}`
    : String(data.span_count)

  const cells: Stat[] = [
    { label: 'Duration', value: formatDuration(data.total_duration_ms) },
    { label: 'Spans', value: spanCount },
    { label: 'Workers', value: String(stats.workers) },
    {
      label: 'Longest span',
      value: stats.longest ? formatDuration(stats.longest.duration_ms) : '—',
      title: stats.longest ? formatSpanLabel(stats.longest) : undefined,
    },
    { label: 'Running', value: String(stats.running) },
    { label: 'Errors', value: String(stats.errors), tone: 'alert' },
  ]

  return (
    <div className="@container shrink-0 font-inter">
      {/* Title row: the worker's colour as the mark, the root span as the
          name, the trace id to copy, and the run's state on the right. */}
      <div className="flex h-11 items-center gap-2 pr-2 pl-3">
        <span
          aria-hidden
          className="size-2 shrink-0 rounded-full"
          style={{ backgroundColor: getWorkerColor(rootWorker) }}
        />
        <h2
          className="min-w-0 flex-1 truncate text-[13px] font-semibold text-foreground"
          title={rootSpan?.name ?? 'Trace details'}
        >
          {title}
          <span className="ml-1.5 text-xs font-normal text-muted-foreground">
            {rootWorker}
          </span>
        </h2>
        <button
          type="button"
          onClick={() => copy('traceId', traceId)}
          title="copy trace id"
          aria-label="copy trace id"
          className="group hidden h-6 shrink-0 items-center gap-1 rounded-md px-1.5 font-mono text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50 @lg:flex"
        >
          <span className="tabular-nums">{traceId.substring(0, 12)}</span>
          {copied ? (
            <Check className="size-4 text-ok" aria-hidden />
          ) : (
            <Copy
              className="size-4 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
              aria-hidden
            />
          )}
        </button>
        <LiveBadge live={live} />
        {chatLink?.turnId && onOpenMessage ? (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={() => onOpenMessage(chatLink)}
            aria-label="go to message"
            title="Open the chat at this trace's message"
          >
            <LocateFixed />
          </Button>
        ) : null}
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          onClick={onClose}
          aria-label="close trace detail"
          title="Close (Esc)"
        >
          <X />
        </Button>
      </div>

      {/* The stats strip: one quiet band, cells divided by hairlines. */}
      <div className="mx-3 mb-2 grid grid-cols-3 divide-x divide-border overflow-hidden rounded-lg border border-border bg-muted/30 @2xl:grid-cols-6">
        {cells.map((cell) => (
          <StatCell key={cell.label} {...cell} />
        ))}
      </div>
    </div>
  )
}
