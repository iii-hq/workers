import { ChevronRight, Clock, Layers, X } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'

/**
 * Loading placeholder for the trace detail — mirrors the real composition
 * (TraceHeader → view switcher → lane timeline → collapsed WorkerBreakdown)
 * block-for-block with the same paddings, borders, and row heights, so the
 * loaded detail resolves in place without a layout jump. Static chrome
 * (icons, section labels, chip frames) renders for real; only the
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

interface TraceDetailSkeletonProps {
  /** wired to the real close affordance so a slow load can be backed out of */
  onClose: () => void
}

export function TraceDetailSkeleton({ onClose }: TraceDetailSkeletonProps) {
  return (
    <div className="flex flex-col overflow-hidden">
      {/* ── TraceHeader ── */}
      <div className="border-b border-rule-2 flex-shrink-0">
        <div className="flex items-center gap-2 px-4 pt-3 pb-1.5">
          <span className="px-1.5 py-0.5 flex-shrink-0 rounded-xs bg-surface">
            <Skeleton className="h-3 w-16" />
          </span>
          <span className="flex-1 min-w-0">
            {/* bg-surface-active where the skeleton sits directly on the panel —
                the default bg-panel tone vanishes there */}
            <Skeleton className="h-3.5 w-44 max-w-full bg-surface-active" />
          </span>
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
          <Skeleton className="h-3 w-[72px] bg-surface-active" />
          <span aria-hidden className="w-px h-3 bg-surface-active-2" />
          <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
            <Clock className="w-2.5 h-2.5 text-ink-faint" />
            <Skeleton className="h-3 w-12" />
          </span>
          <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
            <Layers className="w-2.5 h-2.5 text-ink-faint" />
            <Skeleton className="h-3 w-14" />
          </span>
          <span className="flex items-center gap-1 px-2 py-0.5 rounded-xs bg-surface">
            <Skeleton className="h-3 w-16" />
          </span>
        </div>

        {/* worker share bar + names */}
        <div className="px-4 pb-2.5">
          <div className="flex h-1.5 rounded-full bg-surface overflow-hidden">
            <Skeleton className="h-full w-[52%]" />
            <span aria-hidden className="w-px h-full bg-surface-active-2" />
            <Skeleton className="h-full w-[31%]" />
            <span aria-hidden className="w-px h-full bg-surface-active-2" />
            <Skeleton className="h-full flex-1" />
          </div>
          <div className="flex items-center gap-3 mt-1.5">
            {(['wk-0', 'wk-1', 'wk-2'] as const).map((k) => (
              <span key={k} className="flex items-center gap-1">
                <Skeleton className="w-1.5 h-1.5 flex-shrink-0 bg-surface-active" />
                <Skeleton className="h-2.5 w-16 bg-surface-active" />
              </span>
            ))}
          </div>
        </div>

        {/* critical-path breadcrumb */}
        <div className="flex items-center gap-1 px-4 pb-2.5">
          {(['cp-0', 'cp-1', 'cp-2', 'cp-3'] as const).map((k, i) => (
            <span key={k} className="flex items-center gap-1">
              {i > 0 && (
                <ChevronRight className="w-3 h-3 text-ink-ghost flex-shrink-0" />
              )}
              <span className="px-1 py-0.5">
                <Skeleton className="h-3 w-20 bg-surface-active" />
              </span>
            </span>
          ))}
        </div>
      </div>

      {/* ── view switcher ── */}
      <div className="border-b border-rule-2 px-4 py-2.5">
        <span className="inline-flex rounded-sm bg-surface p-[2px]">
          <span className="px-3 py-1 flex items-center">
            <Skeleton className="h-3.5 w-14" />
          </span>
          <span className="px-3 py-1 flex items-center">
            <Skeleton className="h-3.5 w-16" />
          </span>
        </span>
      </div>

      {/* ── lane timeline (ruler + hierarchical bar cascade) — content-sized,
          like the real canvas (fitContent) ── */}
      <div className="overflow-hidden">
        <div className="relative h-5 mx-4 border-b border-rule-2">
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
              className="absolute h-4"
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
      <div className="border-t border-rule-2 flex-shrink-0">
        <div className="flex items-center gap-2 px-4 py-2.5">
          <ChevronRight className="w-3 h-3 text-ink-faint" />
          <span className="font-mono text-[11px] font-medium uppercase tracking-[0.06em] text-ink-faint">
            workers
          </span>
          <span className="flex items-center gap-3 ml-auto font-mono text-[11px] text-ink-faint">
            {(['p50', 'p95', 'p99'] as const).map((p) => (
              <span key={p} className="flex items-center gap-1">
                {p} <Skeleton className="h-3 w-12 bg-surface-active" />
              </span>
            ))}
          </span>
        </div>
      </div>
    </div>
  )
}
