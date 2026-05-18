// Session detail panel: stacked vertical view of every trace inside a
// session-level group.
//
// When the user clicks a group row in the TRACES tab (with `Group by`
// active), `TraceGroupsView` previously surfaced only the FIRST trace in
// `group.trace_ids`. That caused a confusing UX: the row would say
// "26 spans" but the right-side detail panel would show 4 (just the
// first trace). This component fixes that by rendering one collapsible
// card per trace in the group; each card fetches its own tree on first
// expand (lazy), so opening a 100-trace session no longer fires 100
// simultaneous WebSocket calls.
//
// Fetch strategy:
//
//   Group click
//        │
//        ▼
//   SessionDetailPanel mounts (no network)
//        │
//        ├── TraceCard #1   open=true ──► useQuery enabled ──► fetchTraceTree
//        ├── TraceCard #2   open=false ─┐
//        ├── TraceCard #3   open=false  ├── idle, no network until user clicks
//        └── ...                        ┘
//
//   On user click → setOpen(true) → useQuery enabled → fetchTraceTree
//   On subsequent re-open → TanStack Query cache (30s staleTime) → instant
//
// Each TraceCard reuses the existing render path
// (`treeToWaterfallData` + `WaterfallChart`) so spans behave identically
// to the single-trace view. Span clicks bubble up via `onSpanClick`,
// keeping the right-side span detail panel working unchanged.
//
// Note: the iii-browser-sdk client is resolved internally by
// `fetchTraceTree` via `getIiiClient()`, so this component does not
// take an SDK argument.

import { useQuery } from '@tanstack/react-query'
import { ChevronDown, ChevronRight, Clock, Loader2, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import {
  fetchTraceTree,
  type TraceGroup,
  type TraceTreeResponse,
} from '../api/traces'
import {
  treeToWaterfallData,
  type VisualizationSpan,
  type WaterfallData,
} from '../lib/traceTransform'
import { WaterfallChart } from './WaterfallChart'

interface SessionDetailPanelProps {
  group: TraceGroup
  /**
   * Attribute the parent grouped by, e.g. `iii.message.id` /
   * `iii.session.id` / `iii.function.id`. Used to label the panel
   * header so the user sees "message msg-..." when grouped by message
   * rather than the stale "session msg-..." from this component's
   * session-only origin.
   */
  groupAttribute?: 'iii.message.id' | 'iii.session.id' | 'iii.function.id'
  onClose: () => void
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string
}

function labelFor(attr: SessionDetailPanelProps['groupAttribute']): string {
  switch (attr) {
    case 'iii.message.id':
      return 'message'
    case 'iii.function.id':
      return 'function'
    default:
      return 'session'
  }
}

export function SessionDetailPanel({
  group,
  groupAttribute,
  onClose,
  onSpanClick,
  selectedSpanId,
}: SessionDetailPanelProps) {
  const totalSpans = group.span_count
  const traceCount = group.trace_ids.length
  const errorCount = group.error_count
  const durationSec = (group.duration_ms / 1000).toFixed(2)
  const groupLabel = labelFor(groupAttribute)

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="border-b border-rule px-4 py-3 flex items-center gap-3 flex-shrink-0 bg-panel">
        <div className="flex-1 min-w-0">
          <div className="text-[11px] font-mono font-medium text-ink truncate lowercase">
            {groupLabel} <span className="font-mono">{group.value}</span>
          </div>
          <div className="flex items-center gap-3 mt-0.5 font-mono text-[10px] text-ink-faint lowercase">
            <span className="tabular-nums">
              {traceCount} trace{traceCount === 1 ? '' : 's'}
            </span>
            <span aria-hidden>·</span>
            <span className="tabular-nums">{totalSpans} spans</span>
            <span aria-hidden>·</span>
            <span className="flex items-center gap-1">
              <Clock className="w-3 h-3" />
              <span className="tabular-nums">{durationSec}s</span>
            </span>
            {errorCount > 0 && (
              <>
                <span aria-hidden>·</span>
                <span className="text-alert tabular-nums">
                  {errorCount} error{errorCount === 1 ? '' : 's'}
                </span>
              </>
            )}
          </div>
        </div>
        <Button
          variant="icon"
          size="icon"
          onClick={onClose}
          aria-label="close session detail"
          title="close"
        >
          <X className="w-4 h-4" />
        </Button>
      </div>

      {/* Stacked trace cards. Each owns its own fetch (lazy, on first
          expand) so opening a session with N traces doesn't fire N
          parallel WebSocket calls. The first card auto-opens for small
          traces; large traces require an explicit click so the user
          isn't stuck staring at a "loading…" header that may be slow
          because `engine::traces::tree` is shipping a multi-MB payload.
          The chart itself is virtualized via
          `lib/virtualWindow.ts`, so once data lands the render is
          cheap. The slowness today is end-to-end fetch latency, which
          this panel can't shrink — but it can surface elapsed time so
          users know it's still going. */}
      <div className="flex-1 overflow-y-auto min-h-0">
        {group.trace_ids.map((traceId, idx) => {
          // For single-trace groups, the group's span_count IS the
          // trace's span_count and we can show it as an up-front hint
          // on the closed-card header. For multi-trace groups we don't
          // know the per-trace size until we fetch, so we don't try to
          // guess.
          const expectedSpans = traceCount === 1 ? totalSpans : undefined
          const autoOpen =
            idx === 0 && (expectedSpans === undefined || expectedSpans < AUTO_OPEN_SPAN_LIMIT)
          return (
            <TraceCard
              key={traceId}
              index={idx + 1}
              traceId={traceId}
              onSpanClick={onSpanClick}
              selectedSpanId={selectedSpanId}
              defaultOpen={autoOpen}
              expectedSpans={expectedSpans}
            />
          )
        })}
      </div>
    </div>
  )
}

/**
 * Span count above which the first trace card no longer auto-opens.
 * `engine::traces::tree` returns the whole tree as a single response,
 * and the payload for thousands of spans takes several seconds to
 * serialize, transmit, and parse end-to-end. Auto-firing the query on
 * panel open leaves the user staring at "loading…" with no signal
 * that anything's happening. Below this limit the experience is fast
 * enough to keep the auto-open affordance.
 */
const AUTO_OPEN_SPAN_LIMIT = 500

interface TraceCardProps {
  index: number
  traceId: string
  onSpanClick: (span: VisualizationSpan) => void
  selectedSpanId?: string
  defaultOpen: boolean
  /** Optional hint sourced from the group's span_count for single-trace groups. */
  expectedSpans?: number
}

function TraceCard({
  index,
  traceId,
  onSpanClick,
  selectedSpanId,
  defaultOpen,
  expectedSpans,
}: TraceCardProps) {
  const [open, setOpen] = useState(defaultOpen)

  // Lazy fetch: only enabled when the card has been opened at least
  // once. `staleTime: 30s` keeps re-opens instant; TanStack Query's
  // cache also dedupes if the same trace appears in two groups
  // (different attribute values, overlapping trace_ids). `retry: 0`
  // surfaces errors in a few seconds instead of stacking retry
  // attempts that look identical to "loading forever" to the user.
  // The diagnostic console.log makes it possible to tell at a glance
  // whether the engine actually answered — open devtools and watch
  // the console while waiting.
  const { data, isLoading, error } = useQuery({
    queryKey: ['trace-tree', traceId],
    queryFn: async () => {
      const startedAt = performance.now()
      if (import.meta.env.DEV) {
        console.debug('[traces] fetchTraceTree start', { traceId })
      }
      try {
        const result = await fetchTraceTree(traceId)
        const elapsedMs = Math.round(performance.now() - startedAt)
        if (import.meta.env.DEV) {
          console.debug('[traces] fetchTraceTree done', {
            traceId,
            elapsedMs,
            roots: result.roots?.length ?? 0,
          })
        }
        return result
      } catch (err) {
        const elapsedMs = Math.round(performance.now() - startedAt)
        if (import.meta.env.DEV) {
          console.warn('[traces] fetchTraceTree failed', { traceId, elapsedMs, err })
        }
        throw err
      }
    },
    staleTime: 30_000,
    enabled: open,
    retry: 0,
  })

  // Elapsed seconds while the query is in flight. Ticks at 1s so the
  // user has a signal that the page isn't frozen — useful especially
  // for huge traces where `engine::traces::tree` may take several
  // seconds to respond.
  const [elapsedSec, setElapsedSec] = useState(0)
  const queryStartRef = useRef<number | null>(null)
  useEffect(() => {
    if (!isLoading) {
      queryStartRef.current = null
      setElapsedSec(0)
      return
    }
    queryStartRef.current = performance.now()
    setElapsedSec(0)
    const interval = setInterval(() => {
      if (queryStartRef.current === null) return
      setElapsedSec(Math.floor((performance.now() - queryStartRef.current) / 1000))
    }, 1000)
    return () => clearInterval(interval)
  }, [isLoading])

  const waterfall = data ? buildWaterfall(data) : undefined
  const resolvedSpanCount = waterfall?.span_count ?? expectedSpans ?? 0
  const headerLabel =
    waterfall?.spans[0]?.name ??
    (open ? 'loading…' : expectedSpans ? `click to load (${expectedSpans} spans)` : 'click to load')
  const durationMs = waterfall?.total_duration_ms ?? 0
  const tracePreview = traceId.slice(0, 12)
  const errorMessage =
    error instanceof Error ? error.message : error ? 'failed to load' : null

  return (
    <div className="border-b border-rule">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full px-4 py-2.5 flex items-center gap-2 text-left hover:bg-panel transition-colors"
      >
        {open ? (
          <ChevronDown className="w-3.5 h-3.5 text-ink-faint flex-shrink-0" />
        ) : (
          <ChevronRight className="w-3.5 h-3.5 text-ink-faint flex-shrink-0" />
        )}
        <span className="text-[10px] font-mono text-ink-faint w-6 flex-shrink-0 tabular-nums">
          #{index}
        </span>
        <span className="text-[12px] font-mono truncate text-ink flex-1 lowercase">
          {headerLabel}
        </span>
        <span className="text-[10px] font-mono text-ink-faint flex-shrink-0 tabular-nums">
          {tracePreview}…
        </span>
        <span className="text-[10px] font-mono text-ink-faint flex-shrink-0 w-12 text-right tabular-nums lowercase">
          {resolvedSpanCount > 0 ? `${resolvedSpanCount} sp` : ''}
        </span>
        <span className="text-[10px] font-mono text-ink-faint flex-shrink-0 w-16 text-right tabular-nums lowercase">
          {durationMs > 0 ? `${durationMs.toFixed(0)}ms` : ''}
        </span>
      </button>

      {open && (
        <div className="bg-panel/40">
          {isLoading && (
            <div className="flex items-center justify-center py-6 gap-2 font-mono text-[11px] text-ink-faint lowercase">
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
              <span>
                {expectedSpans
                  ? `loading trace (${expectedSpans} spans)`
                  : 'loading trace'}
                {elapsedSec > 0 ? ` — ${elapsedSec}s` : ''}…
              </span>
            </div>
          )}
          {errorMessage && (
            <div className="px-4 py-3 font-mono text-[11px] text-alert lowercase">
              {errorMessage}
            </div>
          )}
          {waterfall && (
            <WaterfallChart
              data={waterfall}
              onSpanClick={onSpanClick}
              selectedSpanId={selectedSpanId}
            />
          )}
        </div>
      )}
    </div>
  )
}

function buildWaterfall(resp: TraceTreeResponse): WaterfallData | undefined {
  if (!resp.roots || resp.roots.length === 0) return undefined
  const wf = treeToWaterfallData(resp.roots)
  return wf ?? undefined
}
