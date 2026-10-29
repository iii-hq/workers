/**
 * The trace card's top: who ran it and what it was, then the numbers that
 * describe the run at a glance — one quiet line of label·value pairs. On a
 * wide card the numbers ride the title row itself; on a narrower one they
 * drop to a single line under the title (wrapping, never truncating), so
 * the header gives the timeline below as much of the pane as it can.
 *
 * Everything shown is derived from the spans already loaded: duration,
 * span and worker counts, the longest span, how many are still running
 * (the Live badge breathes while any are) and how many failed.
 */

import { Check, Copy, LocateFixed, X } from 'lucide-react'
import { useMemo } from 'react'
import { Button } from '@/components/shadcn/button'
import { Hint } from '@/components/ui/Hint'
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
    <div
      className="flex items-baseline gap-1.5 whitespace-nowrap"
      title={title}
    >
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          'font-mono text-[13px] font-medium tabular-nums',
          hot ? 'text-alert-strong' : 'text-foreground',
        )}
      >
        {value}
      </dd>
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
          name, the stats, the trace id to copy, and the run's state on the
          right. Below @5xl the stats wrap onto their own line under the
          title (order-last + full basis), indented to the title's text. */}
      <div className="flex min-h-10 flex-wrap items-center gap-x-2 py-1.5 pr-2 pl-3">
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
        <dl
          aria-label="trace stats"
          className="order-last flex basis-full flex-wrap items-baseline gap-x-4 gap-y-0.5 pt-0.5 pl-4 @5xl:order-none @5xl:shrink-0 @5xl:basis-auto @5xl:flex-nowrap @5xl:pt-0 @5xl:pr-2 @5xl:pl-0"
        >
          {cells.map((cell) => (
            <StatCell key={cell.label} {...cell} />
          ))}
        </dl>
        <Hint label="Copy trace id">
          <button
            type="button"
            onClick={() => copy('traceId', traceId)}
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
        </Hint>
        <LiveBadge live={live} />
        {chatLink?.turnId && onOpenMessage ? (
          <Hint label="Open the chat at this trace's message">
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              onClick={() => onOpenMessage(chatLink)}
              aria-label="go to message"
            >
              <LocateFixed />
            </Button>
          </Hint>
        ) : null}
        <Hint label="Close (Esc)">
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            onClick={onClose}
            aria-label="close trace detail"
          >
            <X />
          </Button>
        </Hint>
      </div>
    </div>
  )
}
