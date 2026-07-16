// Span-filter fallout on the trace LIST: hide rows whose trace is
// completely composed of hidden spans.
//
// The funnel selection (`useSpanFilterSelection`) already de-noises the
// strip and the detail views span-by-span; this hook extends it to the
// list, so a trace with NO visible span left — session bookkeeping once
// its family is hidden, the default-hidden internal fan-outs — stops
// occupying a row (its detail would render empty anyway). A trace keeps
// its row while ANY of its spans survives the filter. Root-only matching
// would be wrong in both directions: turn traces are ROOTED in the
// producer-default-hidden `harness::turn` dispatch group (a root-only rule
// would wipe real work off the list), and a visible root over a fully
// hidden subtree is still a visible bar.
//
// Verdicts (traceId → "has a visible span") come from three sources, in
// order:
//
// 1. the row's OWN root span — visible root ⇒ visible trace, no more data
//    needed (the common case: most rows are rooted in unhidden work);
// 2. the all-spans feed (`useAllSpans`) — the spans the strip draws;
// 3. a one-shot batched read of the trace's stored spans for hidden-rooted
//    rows the feed doesn't cover (the feed retains ~2min; the list's 500
//    rows reach much further back).
//
// Both span sources count only what the DETAIL views can render: engine
// builtin spans (`include_internal: false` drops them from the detail
// read) never keep a row alive, so a kept row always opens to a non-empty
// detail.
//
// Under a FIXED selection visibility is MONOTONE: spans only accumulate
// in the store, so a trace with one visible span has it forever. A `true`
// verdict therefore sticks; only unknown traces start at `false` and flip
// the moment a surviving span arrives (the first frames of a turn are all
// dispatch plumbing). Without the stick, a long run would vanish mid-way:
// the feed retains ~2min, so a quiet stretch (one long LLM/tool call)
// prunes the visible bars while hidden bookkeeping keeps the trace in the
// feed — and a feed-only recompute would flip the row back to hidden.
// Fetched verdicts land in the same cache. The cache resets when the
// selection changes; a row stays visible while its verdict is unknown or
// its read failed — better to show a hideable row than to hide real work
// on a guess.
//
// LIVE traces get one more guard: worker spans reach the store only when
// they CLOSE, so a running turn's composition is structurally incomplete —
// its first visible span may be an LLM call that takes minutes to close.
// A negative verdict is a guess there, so a row whose trace is still live
// (pending span in the feed, or a span end within the last few seconds)
// stays visible regardless. Bookkeeping traces settle in well under a
// second, so the exemption never resurfaces them.

import { useEffect, useMemo, useRef, useState } from 'react'
import { fetchTraces, type StoredSpan } from '../api/traces'
import type { TimelineSpan } from '../components/timeline/layout'
import {
  isSpanBarHidden,
  type SpanFilterKeys,
} from '../components/timeline/spanVisibility'
import type { SpanFilterSelection } from '../lib/spanFilters'
import { internalFamilyOf } from '../lib/spanLabel'
import {
  spanFilterGroupKey,
  storedSpansToTimelineSpans,
} from '../lib/timelineSpans'
import { isPendingSpan, toMs } from '../lib/traceTransform'
import { getWorkerName } from '../lib/traceUtils'
import type { TraceListItem } from './useTraceData'

/** Traces per composition read — small enough that `FETCH_SPAN_LIMIT`
 *  practically never truncates (bookkeeping traces run a handful of spans;
 *  turn traces a few hundred). */
const FETCH_TRACE_CHUNK = 20
const FETCH_SPAN_LIMIT = 10_000

/** How recently a span of the trace must have ENDED for the trace to still
 *  count as live when no pending snapshot is in the feed (engines with
 *  `live_spans` off). Generous on purpose — liveness only defers hiding. */
const LIVE_END_SLACK_MS = 10_000

/**
 * Traces with work still in flight, judged from the raw feed: a pending
 * snapshot (the engine's queue wrapper stays pending for a turn's whole
 * lifetime — see `followTurn.ts`), or a span that ended moments ago.
 * Exported for tests.
 */
export function liveTraceIds(
  spans: readonly StoredSpan[],
  now: number,
): ReadonlySet<string> {
  const live = new Set<string>()
  for (const span of spans) {
    if (live.has(span.trace_id)) continue
    if (
      isPendingSpan(span) ||
      now - toMs(span.end_time_unix_nano) <= LIVE_END_SLACK_MS
    ) {
      live.add(span.trace_id)
    }
  }
  return live
}

/**
 * The row's ROOT span rendered as filter keys, mirroring how
 * `storedSpansToTimelineSpans` keys the same span in the feed (a listed
 * root has no ancestry, so the inherited tag kind is undefined).
 * Exported for tests.
 */
export function rowRootFilterKeys(row: TraceListItem): SpanFilterKeys {
  const attrs = row.attributes ?? {}
  // `mapSpanToListItem` collapses a missing service_name to the 'unknown'
  // sentinel; undo it so the worker key falls back exactly like the feed's.
  const service = row.workers[0] === 'unknown' ? undefined : row.workers[0]
  return {
    groupKey: spanFilterGroupKey(row.rootOperation, attrs, undefined),
    workerKey: getWorkerName({
      service_name: service,
      name: row.rootOperation,
    }),
    internalKey: internalFamilyOf(attrs) ?? undefined,
  }
}

/**
 * One reconcile pass: refresh `verdicts` from the bars currently in the
 * feed, seed visible-root verdicts from the rows themselves, drop entries
 * for traces gone from both the feed and the list, and return the rows
 * that survive (unknown verdicts stay visible, and so do rows in
 * `liveTraces` — a running trace's negative verdict is a guess, its
 * visible spans may simply not have closed yet). Verdicts are monotone
 * within a selection: `true` sticks, `false` re-evaluates. Mutates
 * `verdicts` — the hook owns the map across renders. Exported for tests.
 */
export function reconcileTraceVisibility(
  verdicts: Map<string, boolean>,
  bars: readonly TimelineSpan[],
  rows: readonly TraceListItem[],
  selection: SpanFilterSelection,
  liveTraces: ReadonlySet<string> = new Set(),
): readonly TraceListItem[] {
  const inFeed = new Set<string>()
  for (const bar of bars) {
    const traceId = bar.traceId
    if (!traceId) continue
    if (!inFeed.has(traceId)) {
      inFeed.add(traceId)
      // Monotone: a known-visible trace stays visible (spans only
      // accumulate); only unknown traces start hidden pending a
      // surviving bar.
      if (!verdicts.has(traceId)) verdicts.set(traceId, false)
    }
    if (!verdicts.get(traceId) && !isSpanBarHidden(bar, selection)) {
      verdicts.set(traceId, true)
    }
  }
  // A visible root IS a visible span — worth recording even over a feed
  // verdict of hidden, since the feed may hold only a partial tail of an
  // older trace while the row carries the actual root.
  for (const row of rows) {
    if (verdicts.get(row.traceId)) continue
    if (!isSpanBarHidden(rowRootFilterKeys(row), selection)) {
      verdicts.set(row.traceId, true)
    }
  }
  const listed = new Set(rows.map((r) => r.traceId))
  for (const traceId of [...verdicts.keys()]) {
    if (!inFeed.has(traceId) && !listed.has(traceId)) {
      verdicts.delete(traceId)
    }
  }
  const kept = rows.filter(
    (r) => verdicts.get(r.traceId) !== false || liveTraces.has(r.traceId),
  )
  // Identity-stable when nothing is hidden, so downstream memos hold.
  return kept.length === rows.length ? rows : kept
}

/**
 * Mirrors the engine's `include_internal` exclusion in `traces::list`:
 * spans whose own attributes mark engine machinery/builtins (`call
 * state::get`, `engine::*`). The DETAIL views read with
 * `include_internal: false`, so these spans never render there — a trace
 * whose only surviving spans are builtins would still open empty on
 * click, and must not keep its row. (They stay in the CONVERSION input so
 * parent chains resolve; they just never count as visible.)
 */
function isEngineInternalStoredSpan(span: StoredSpan): boolean {
  return span.attributes.some(
    ([k, v]) =>
      (k === 'iii.function.kind' && v === 'internal') ||
      (k === 'function_id' &&
        typeof v === 'string' &&
        v.startsWith('engine::')),
  )
}

/** Bars that count toward "the trace still shows something": converted
 *  with full parent chains, then stripped of engine builtins — exactly
 *  what the detail views can render. */
function verdictBars(spans: readonly StoredSpan[]): TimelineSpan[] {
  const builtins = new Set(
    spans.filter(isEngineInternalStoredSpan).map((s) => s.span_id),
  )
  const bars = storedSpansToTimelineSpans(spans)
  return builtins.size === 0 ? bars : bars.filter((b) => !builtins.has(b.id))
}

/**
 * Fold one composition read into the verdicts: every requested trace's
 * verdict is "any of its bars survives the filter" — a trace the store no
 * longer has spans for counts as hidden (its detail would be empty). A
 * truncated response (span count at the limit) only trusts POSITIVE
 * verdicts: a visible span proves visibility, but "all hidden" might just
 * mean the visible spans were cut off. A `true` verdict already in the
 * cache is never downgraded — the read raced a feed frame that proved
 * visibility after the snapshot was taken. Exported for tests.
 */
export function mergeFetchedVerdicts(
  verdicts: Map<string, boolean>,
  requested: readonly string[],
  spans: readonly StoredSpan[],
  selection: SpanFilterSelection,
  spanLimit: number = FETCH_SPAN_LIMIT,
): void {
  const visible = new Set<string>()
  for (const bar of verdictBars(spans)) {
    if (bar.traceId && !isSpanBarHidden(bar, selection)) {
      visible.add(bar.traceId)
    }
  }
  const truncated = spans.length >= spanLimit
  for (const traceId of requested) {
    if (visible.has(traceId)) verdicts.set(traceId, true)
    else if (!truncated && !verdicts.get(traceId)) {
      verdicts.set(traceId, false)
    }
  }
}

export function useSpanFilteredTraceRows(
  rows: readonly TraceListItem[],
  feedSpans: readonly StoredSpan[],
  selection: SpanFilterSelection,
): readonly TraceListItem[] {
  // The strip's own mapping (routing wrappers skipped, filter keys
  // resolved) minus engine builtins, run on the UNPRUNED feed so verdicts
  // reach the full retention window rather than the strip's visible one.
  const bars = useMemo(() => verdictBars(feedSpans), [feedSpans])

  // Verdict cache, keyed by selection IDENTITY: `useSpanFilterSelection`
  // memoizes its controls object on the effective selection, so a new
  // reference means the selection changed and every cached verdict is
  // stale. `requested` remembers which traces already went through a
  // composition read (in-flight, done, or failed) so they are asked once
  // per selection.
  const cacheRef = useRef<{
    selection: SpanFilterSelection
    verdicts: Map<string, boolean>
    requested: Set<string>
  } | null>(null)

  // Bumped when a composition read lands verdicts, to re-run the memo.
  const [fetchTick, setFetchTick] = useState(0)

  // fetchTick isn't read in the body — it signals `cache.verdicts` grew.
  // The liveness set is derived from the feed at reconcile time (feed
  // frames arrive continuously while anything is live, so it stays fresh
  // without its own clock).
  // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  const visibleRows = useMemo(() => {
    let cache = cacheRef.current
    if (!cache || cache.selection !== selection) {
      cache = { selection, verdicts: new Map(), requested: new Set() }
      cacheRef.current = cache
    }
    return reconcileTraceVisibility(
      cache.verdicts,
      bars,
      rows,
      selection,
      liveTraceIds(feedSpans, Date.now()),
    )
  }, [rows, bars, feedSpans, selection, fetchTick])

  // Composition reads for the rows neither source could judge: hidden
  // root, no feed coverage. Runs after the memo above, so feed verdicts
  // are already in the cache. Deliberately NO cancellation on dep churn —
  // live row appends must not orphan an in-flight read (its traces would
  // stay `requested` but never verdicted); a stale SELECTION is detected
  // by cache identity instead. Failed reads leave their traces visible
  // and unretried until the selection changes.
  useEffect(() => {
    const cache = cacheRef.current
    if (!cache || cache.selection !== selection) return
    const unknown = rows
      .filter(
        (r) =>
          !cache.verdicts.has(r.traceId) && !cache.requested.has(r.traceId),
      )
      .map((r) => r.traceId)
    if (unknown.length === 0) return
    for (const traceId of unknown) cache.requested.add(traceId)
    void (async () => {
      for (let i = 0; i < unknown.length; i += FETCH_TRACE_CHUNK) {
        const chunk = unknown.slice(i, i + FETCH_TRACE_CHUNK)
        try {
          const res = await fetchTraces({
            trace_ids: chunk,
            search_all_spans: true,
            include_internal: true,
            limit: FETCH_SPAN_LIMIT,
          })
          if (cacheRef.current !== cache) return
          mergeFetchedVerdicts(cache.verdicts, chunk, res.spans, selection)
          setFetchTick((t) => t + 1)
        } catch {
          // Traces unavailable (memory exporter off, transient error) —
          // the rows simply stay visible.
        }
      }
    })()
  }, [rows, selection])

  return visibleRows
}
