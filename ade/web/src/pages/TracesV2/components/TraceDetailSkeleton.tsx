import { ChevronRight, LocateFixed, X } from 'lucide-react'
import { Button } from '@/components/shadcn/button'
import { Skeleton } from '@/components/ui/Skeleton'
import type { TraceChatLink } from '../lib/traceChatLink'

/**
 * Loading placeholder for the trace detail — mirrors the real composition
 * (TraceHeader → stats strip → view toolbar → lane timeline → collapsed
 * WorkerBreakdown) block-for-block with the same paddings, borders, and
 * row heights, so the loaded detail resolves in place without a layout
 * jump. Static chrome (icons, section labels) renders for real; only the
 * data-dependent values pulse.
 */

/** the fake lane cascade: [left%, width%] per 22px-pitch line, root first */
const TIMELINE_BARS: ReadonlyArray<readonly [number, number]> = [
  [0, 97],
  [2, 41],
  [5, 24],
  [31, 11],
  [45, 26],
  [48, 13],
  [63, 7],
  [74, 22],
]

const STAT_CELLS = ['sk-0', 'sk-1', 'sk-2', 'sk-3', 'sk-4', 'sk-5'] as const

interface TraceDetailSkeletonProps {
  /** wired to the real close affordance so a slow load can be backed out of */
  onClose: () => void
  /** chat linkage already resolved from the row's trace tags — live like the
   *  close button, so the jump to the conversation never waits on the spans */
  chatLink?: TraceChatLink | null
  onOpenMessage?: (link: TraceChatLink) => void
}

export function TraceDetailSkeleton({
  onClose,
  chatLink,
  onOpenMessage,
}: TraceDetailSkeletonProps) {
  return (
    <div
      role="status"
      aria-busy="true"
      aria-label="loading trace"
      className="@container flex flex-col overflow-hidden font-inter"
    >
      {/* ── TraceHeader: title row ── */}
      <div className="flex h-11 items-center gap-2 pr-2 pl-3">
        <Skeleton className="size-2 rounded-full" />
        <Skeleton className="h-3.5 w-44 max-w-full" />
        <span className="flex-1" />
        <Skeleton className="h-6 w-20 rounded-md" />
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

      {/* ── stats strip ── */}
      <div className="mx-3 mb-2 grid grid-cols-3 divide-x divide-border overflow-hidden rounded-lg border border-border bg-muted/30 @2xl:grid-cols-6">
        {STAT_CELLS.map((k) => (
          <div key={k} className="flex flex-col gap-1.5 px-3 py-2">
            <Skeleton className="h-2.5 w-12" />
            <Skeleton className="h-4 w-14" />
          </div>
        ))}
      </div>

      {/* ── view toolbar ── */}
      <div className="flex h-9 items-center gap-2 border-b border-border px-3">
        <Skeleton className="h-7 w-44 rounded-md" />
        <span className="flex-1" />
        <Skeleton className="h-3 w-16" />
      </div>

      {/* ── lane timeline (ruler + hierarchical bar cascade) — content-sized,
          like the real canvas (fitContent) ── */}
      <div className="overflow-hidden">
        <div className="relative mx-4 h-5 border-b border-border">
          {([0, 25, 50, 75] as const).map((pct) => (
            <Skeleton
              key={pct}
              className="absolute top-1.5 h-2.5 w-9"
              style={{ left: `${pct}%` }}
            />
          ))}
          <Skeleton className="absolute top-1.5 right-0 h-2.5 w-9" />
        </div>
        <div
          className="relative mx-4 my-2"
          style={{ height: (TIMELINE_BARS.length - 1) * 22 + 16 }}
        >
          {TIMELINE_BARS.map(([left, width], line) => (
            <Skeleton
              key={`${left}-${width}`}
              className="absolute h-4 rounded-[4px]"
              style={{
                left: `${left}%`,
                width: `${width}%`,
                top: line * 22,
              }}
            />
          ))}
        </div>
      </div>

      {/* ── collapsed workers footer ── */}
      <div className="flex h-9 items-center gap-2 border-t border-border px-3">
        <ChevronRight className="size-4 text-muted-foreground" aria-hidden />
        <span className="text-xs font-semibold text-foreground">Workers</span>
        <span className="ml-auto flex items-center gap-3 text-[11px] text-muted-foreground">
          {(['p50', 'p95', 'p99'] as const).map((p) => (
            <span key={p} className="flex items-center gap-1">
              {p} <Skeleton className="h-3 w-12" />
            </span>
          ))}
        </span>
      </div>
    </div>
  )
}
