/**
 * The worker legend under the detail timeline's toolbar: one chip per
 * worker in the trace, busiest first — the worker's swatch and its name,
 * so the hue on a bar resolves to a name without hovering. Each chip
 * toggles that worker's spans (the same hide the funnel's Workers section
 * applies), and a hidden worker's chip stays, dimmed with an eye-off, so
 * it can be turned back on from here.
 *
 * Colour is never the only cue: the name is always there, and a worker
 * that had to reuse a hue (the ninth worker onward, see `traceColors`)
 * wears a hollow swatch instead of a filled one — the same mark its bars
 * carry on their edge. Past `LEGEND_LIMIT` workers the rest fold behind a
 * "+N" chip so the row stays a row.
 */

import { EyeOff } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'
import {
  borderOnFill,
  type WorkerSwatch,
  workerSwatch,
} from '../../lib/traceColors'
import type { SpanGroup } from './spanVisibility'

/** Chips shown before the rest fold behind "+N". */
export const LEGEND_LIMIT = 10

/** The swatch as a 8px mark: filled for a first-use hue, hollow for a reused one. */
export function WorkerSwatchDot({
  swatch,
  className,
}: {
  swatch: WorkerSwatch
  className?: string
}) {
  return (
    <span
      aria-hidden
      data-worker-swatch={swatch.reused ? 'reused' : 'solid'}
      className={cn('size-2 shrink-0 rounded-full', className)}
      style={
        swatch.reused
          ? { boxShadow: `inset 0 0 0 2px ${swatch.color}` }
          : {
              backgroundColor: swatch.color,
              // The hue's `*-800` edge keeps the dark `*-900` fill
              // readable on a dark panel.
              boxShadow: `inset 0 0 0 1px ${borderOnFill(swatch.color)}`,
            }
      }
    />
  )
}

export interface WorkerLegendProps {
  /** Worker entries, busiest first (`deriveSpanGroups` + `workerGroupKey`). */
  workers: readonly SpanGroup[]
  /** Worker names currently hidden by the span filter. */
  hidden?: ReadonlySet<string>
  /** Toggle a worker's spans; absent renders the legend read-only. */
  onToggle?: (worker: string) => void
  className?: string
}

export function WorkerLegend({
  workers,
  hidden,
  onToggle,
  className,
}: WorkerLegendProps) {
  const [showAll, setShowAll] = useState(false)
  if (workers.length === 0) return null
  const shown = showAll ? workers : workers.slice(0, LEGEND_LIMIT)
  const folded = workers.length - shown.length

  return (
    <ul
      aria-label="workers in this trace"
      data-worker-legend
      className={cn(
        'flex flex-wrap items-center gap-1 px-3 py-1.5 font-inter',
        className,
      )}
    >
      {shown.map((worker) => {
        const swatch = workerSwatch(worker.key)
        const isHidden = hidden?.has(worker.key) ?? false
        const label = `${worker.key} · ${worker.count} ${worker.count === 1 ? 'span' : 'spans'}`
        const chipClass = cn(
          'inline-flex h-6 max-w-[14rem] items-center gap-1.5 rounded-md px-1.5 text-xs text-foreground/80 transition-colors',
          isHidden && 'opacity-60',
        )
        const body = (
          <>
            <WorkerSwatchDot swatch={swatch} />
            <span className="truncate" title={worker.key}>
              {worker.key}
            </span>
            <span className="font-mono text-xs text-muted-foreground tabular-nums">
              {worker.count}
            </span>
            {isHidden ? (
              <EyeOff className="size-4 shrink-0" aria-hidden />
            ) : null}
          </>
        )
        return (
          <li key={worker.key} className="flex min-w-0">
            {onToggle ? (
              <button
                type="button"
                aria-pressed={!isHidden}
                aria-label={
                  isHidden ? `show ${label} (hidden)` : `hide ${label} spans`
                }
                title={
                  isHidden
                    ? `hidden — click to show ${worker.key}`
                    : `hide ${worker.key} spans (their children stay, re-attached to the parent)`
                }
                onClick={() => onToggle(worker.key)}
                data-worker-chip={worker.key}
                className={cn(
                  chipClass,
                  'hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50',
                )}
              >
                {body}
              </button>
            ) : (
              <span data-worker-chip={worker.key} className={chipClass}>
                {body}
              </span>
            )}
          </li>
        )
      })}
      {folded > 0 ? (
        <li className="flex">
          <button
            type="button"
            onClick={() => setShowAll(true)}
            aria-label={`show ${folded} more workers`}
            className="inline-flex h-6 items-center rounded-md px-1.5 font-mono text-[11px] text-muted-foreground tabular-nums hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            +{folded}
          </button>
        </li>
      ) : showAll && workers.length > LEGEND_LIMIT ? (
        <li className="flex">
          <button
            type="button"
            onClick={() => setShowAll(false)}
            className="inline-flex h-6 items-center rounded-md px-1.5 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            Fewer
          </button>
        </li>
      ) : null}
    </ul>
  )
}
