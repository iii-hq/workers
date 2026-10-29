/**
 * The worker legend in the detail timeline's toolbar: one chip per worker
 * in the trace, busiest first — the worker's swatch and its name, so the
 * hue on a bar resolves to a name without hovering. Each chip toggles that
 * worker's spans (the same hide the funnel's Workers section applies), and
 * a hidden worker's chip stays, dimmed with an eye-off, so it can be
 * turned back on from here.
 *
 * Colour is never the only cue: the name is always there, and a worker
 * that had to reuse a hue (the ninth worker onward, see `traceColors`)
 * wears a hollow swatch instead of a filled one — the same mark its bars
 * carry on their edge.
 *
 * The legend is always ONE row: it measures its chips against the width it
 * has (an invisible copy of every chip, re-measured on resize) and folds
 * the workers that don't fit behind a "+N" menu, where each stays a
 * toggle. So it can share the toolbar row instead of wrapping into lines
 * that push the timeline down.
 */

import { EyeOff } from 'lucide-react'
import { useLayoutEffect, useRef, useState } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { cn } from '@/lib/utils'
import {
  borderOnFill,
  type WorkerSwatch,
  workerSwatch,
} from '../../lib/traceColors'
import type { SpanGroup } from './spanVisibility'

/** the chips' flex gap (`gap-1`) */
const CHIP_GAP_PX = 4
/** the list's horizontal padding (`p-0.5`, both sides) */
const LIST_INSET_PX = 4

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

const CHIP_CLASS =
  'inline-flex h-6 max-w-[14rem] shrink-0 items-center gap-1.5 rounded-md px-1.5 text-xs text-foreground/80 transition-colors'

function ChipBody({
  worker,
  isHidden,
}: {
  worker: SpanGroup
  isHidden: boolean
}) {
  return (
    <>
      <WorkerSwatchDot swatch={workerSwatch(worker.key)} />
      <span className="truncate" title={worker.key}>
        {worker.key}
      </span>
      <span className="font-mono text-xs text-muted-foreground tabular-nums">
        {worker.count}
      </span>
      {isHidden ? <EyeOff className="size-4 shrink-0" aria-hidden /> : null}
    </>
  )
}

/**
 * How many leading chips fit in `available` px, leaving room for the "+N"
 * trigger whenever any chip has to fold.
 */
function countFitting(
  widths: readonly number[],
  moreWidth: number,
  available: number,
): number {
  let used = 0
  for (let i = 0; i < widths.length; i++) {
    used += (i > 0 ? CHIP_GAP_PX : 0) + widths[i]
    const last = i === widths.length - 1
    if (used + (last ? 0 : CHIP_GAP_PX + moreWidth) > available) return i
  }
  return widths.length
}

export function WorkerLegend({
  workers,
  hidden,
  onToggle,
  className,
}: WorkerLegendProps) {
  const listRef = useRef<HTMLUListElement>(null)
  const measureRef = useRef<HTMLDivElement>(null)
  const [fit, setFit] = useState(workers.length)

  // Measure the invisible copy (every chip, then a "+N" probe) against the
  // list's width; re-run when the chips change (a hidden worker's chip
  // grows an eye-off) or the row resizes.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the chips are the re-measure trigger, read off the DOM copy rather than these values.
  useLayoutEffect(() => {
    const list = listRef.current
    const measure = measureRef.current
    if (!list || !measure) return
    const compute = () => {
      const nodes = Array.from(measure.children) as HTMLElement[]
      const widths = nodes.slice(0, -1).map((n) => n.offsetWidth)
      const moreWidth = nodes[nodes.length - 1]?.offsetWidth ?? 0
      setFit(countFitting(widths, moreWidth, list.clientWidth - LIST_INSET_PX))
    }
    compute()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(compute)
    observer.observe(list)
    return () => observer.disconnect()
  }, [workers, hidden])

  if (workers.length === 0) return null
  const shown = workers.slice(0, fit)
  const folded = workers.slice(fit)

  return (
    <div className={cn('relative min-w-0 font-inter', className)}>
      {/* the measuring copy: a zero-height clip so its full-width row
          never widens the page, while each chip keeps its natural width */}
      <div
        aria-hidden
        className="pointer-events-none invisible absolute inset-x-0 top-0 h-0 overflow-hidden"
      >
        <div ref={measureRef} className="flex w-max">
          {workers.map((worker) => (
            <span key={worker.key} className={CHIP_CLASS}>
              <ChipBody
                worker={worker}
                isHidden={hidden?.has(worker.key) ?? false}
              />
            </span>
          ))}
          <span className="inline-flex h-6 items-center px-1.5 font-mono text-xs tabular-nums">
            +{workers.length}
          </span>
        </div>
      </div>
      <ul
        ref={listRef}
        aria-label="workers in this trace"
        data-worker-legend
        // the inset keeps the chips' focus rings clear of the clip
        className="-m-0.5 flex items-center gap-1 overflow-hidden p-0.5"
      >
        {shown.map((worker) => {
          const isHidden = hidden?.has(worker.key) ?? false
          const label = `${worker.key} · ${worker.count} ${worker.count === 1 ? 'span' : 'spans'}`
          const chipClass = cn(CHIP_CLASS, isHidden && 'opacity-60')
          const body = <ChipBody worker={worker} isHidden={isHidden} />
          return (
            <li key={worker.key} className="flex min-w-0 shrink-0">
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
        {folded.length > 0 ? (
          <li className="flex shrink-0">
            <FoldedWorkersMenu
              workers={folded}
              hidden={hidden}
              onToggle={onToggle}
            />
          </li>
        ) : null}
      </ul>
    </div>
  )
}

/** The workers that didn't fit the row, still one toggle each. */
function FoldedWorkersMenu({
  workers,
  hidden,
  onToggle,
}: {
  workers: readonly SpanGroup[]
  hidden?: ReadonlySet<string>
  onToggle?: (worker: string) => void
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="xs"
            aria-label={`show ${workers.length} more workers`}
            className="rounded-md px-1.5 font-mono text-xs text-muted-foreground tabular-nums"
          />
        }
      >
        +{workers.length}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="max-h-72 w-64 overflow-y-auto overscroll-contain rounded-lg"
      >
        <DropdownMenuGroup>
          {workers.map((worker) => {
            const isHidden = hidden?.has(worker.key) ?? false
            const row = (
              <>
                <WorkerSwatchDot swatch={workerSwatch(worker.key)} />
                <span className="min-w-0 flex-1 truncate">{worker.key}</span>
                <span className="ml-3 shrink-0 font-mono text-xs text-muted-foreground tabular-nums">
                  {worker.count}
                </span>
              </>
            )
            return onToggle ? (
              <DropdownMenuCheckboxItem
                key={worker.key}
                checked={isHidden}
                onCheckedChange={() => onToggle(worker.key)}
                closeOnClick={false}
                // Checked means HIDDEN, as in the span filter menu.
                indicator={<EyeOff aria-hidden className="size-4" />}
                title={
                  isHidden
                    ? `hidden — click to show ${worker.key}`
                    : `hide ${worker.key} spans (their children stay, re-attached to the parent)`
                }
                data-worker-chip={worker.key}
                className={cn(isHidden && 'text-muted-foreground')}
              >
                {row}
              </DropdownMenuCheckboxItem>
            ) : (
              <DropdownMenuItem key={worker.key} data-worker-chip={worker.key}>
                {row}
              </DropdownMenuItem>
            )
          })}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
