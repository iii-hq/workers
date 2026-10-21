/**
 * The trace card's top: who ran it and what it was, then the numbers that
 * describe the run at a glance — a stats strip of label-over-value cells,
 * the way an observability card reads before its waterfall.
 *
 * Everything shown is derived from the spans already loaded: duration,
 * span and worker counts, the longest span, how many are still running
 * (the Live badge pulses while any are) and how many failed.
 */

import { Check, Copy, LocateFixed, Sparkles, X } from 'lucide-react'
import { useMemo } from 'react'
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

const GHOST_ACTION =
  'inline-flex size-8 shrink-0 items-center justify-center rounded-xl text-neutral-500 transition-colors duration-[var(--motion-duration-control)] hover:bg-neutral-200/70 hover:text-neutral-900 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-500 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-100 dark:focus-visible:ring-neutral-400'

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
    <div className="flex min-w-0 flex-col gap-1 px-4 py-3" title={title}>
      <span className="truncate text-xs font-medium text-neutral-500 dark:text-neutral-400">
        {label}
      </span>
      <span
        className={cn(
          'truncate text-lg font-semibold leading-6 tracking-tight tabular-nums',
          hot
            ? 'text-red-600 dark:text-red-400'
            : 'text-neutral-900 dark:text-neutral-50',
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
      <div className="flex h-12 items-center gap-2.5 px-4">
        <span
          aria-hidden
          className="flex size-6 shrink-0 items-center justify-center rounded-md"
          style={{
            backgroundColor: `color-mix(in oklab, ${getWorkerColor(rootWorker)} 18%, transparent)`,
            color: getWorkerColor(rootWorker),
          }}
        >
          <Sparkles className="size-4" />
        </span>
        <span className="shrink-0 text-sm font-semibold text-neutral-900 dark:text-neutral-50">
          Trace
        </span>
        <h2
          className="min-w-0 flex-1 truncate text-sm text-neutral-500 dark:text-neutral-400"
          title={rootSpan?.name ?? 'Trace details'}
        >
          {title}
        </h2>
        <button
          type="button"
          onClick={() => copy('traceId', traceId)}
          title="copy trace id"
          className="group hidden shrink-0 items-center gap-1 rounded-lg px-1.5 py-0.5 font-mono text-[11px] text-neutral-400 transition-colors hover:bg-neutral-200/70 hover:text-neutral-900 @lg:flex dark:text-neutral-500 dark:hover:bg-neutral-800 dark:hover:text-neutral-100"
        >
          <span className="tabular-nums">{traceId.substring(0, 12)}</span>
          {copied ? (
            <Check className="size-4 text-emerald-500" aria-hidden />
          ) : (
            <Copy
              className="size-4 opacity-0 transition-opacity group-hover:opacity-100"
              aria-hidden
            />
          )}
        </button>
        <span
          role="status"
          className={cn(
            'inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium',
            live
              ? 'bg-sky-500/10 text-sky-600 dark:text-sky-400'
              : 'bg-neutral-200/70 text-neutral-600 dark:bg-neutral-800 dark:text-neutral-300',
          )}
        >
          <span
            aria-hidden
            className={cn(
              'size-1.5 rounded-full',
              live ? 'animate-pulse bg-sky-500' : 'bg-neutral-400',
            )}
          />
          {live ? 'Live' : 'Complete'}
        </span>
        {chatLink?.turnId && onOpenMessage ? (
          <button
            type="button"
            onClick={() => onOpenMessage(chatLink)}
            aria-label="go to message"
            title="open the chat at this trace's message"
            className={GHOST_ACTION}
          >
            <LocateFixed className="size-4" aria-hidden />
          </button>
        ) : null}
        <button
          type="button"
          onClick={onClose}
          aria-label="close trace detail"
          title="close (esc)"
          className={GHOST_ACTION}
        >
          <X className="size-4" aria-hidden />
        </button>
      </div>

      {/* The stats strip: one quiet band, cells divided by hairlines. */}
      <div className="mx-4 mb-3 grid grid-cols-3 divide-x divide-neutral-200/80 rounded-xl bg-neutral-100 @2xl:grid-cols-6 dark:divide-neutral-800 dark:bg-neutral-900">
        {cells.map((cell) => (
          <StatCell key={cell.label} {...cell} />
        ))}
      </div>
    </div>
  )
}
