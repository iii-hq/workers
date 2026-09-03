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
//    rows the feed doesn't cover (the feed retains ~2min; paged history
//    can reach much further back).
//
// Failed rows always survive this composition filter even when every span is
// hidden. Otherwise an early failure in a producer-default-hidden root (for
// example `harness::send` failing before it creates a child) disappears from
// the only surface where the user can diagnose it.
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
// selection changes.
//
// A row with a hidden root and NO verdict yet stays HIDDEN until a read or
// the feed proves visible work. The other default — show while unknown —
// made every hidden bookkeeping trace (state polls, function-change
// fan-outs) flash into the live list and vanish a moment later when its
// composition read landed, which under a busy engine read as rows switching
// states all over the page (MOT-4621). The cost is a short delay before a
// hidden-rooted trace with real work under it earns its row; the common
// turn shape is rooted visibly and never waits.
//
// RUNNING traces are the exception: worker spans reach the store only when
// they CLOSE, so a running turn's composition is structurally incomplete —
// its first visible span may be an LLM call that takes minutes to close.
// A row whose trace is still running (the row's own `pending` status, or a
// pending snapshot in the feed) stays visible regardless, and a composition
// read that finds the trace still running records NO verdict: the read is
// repeated once the row settles, so the eventual verdict sees the closed
// work. Bookkeeping traces are already complete when they are listed, so
// the exemption never resurfaces them.

import { useEffect, useMemo, useRef, useState } from 'react'
import { fetchTraceSpans, type StoredSpan } from '../api/traces'
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
import { isPendingSpan } from '../lib/traceTransform'
import { getWorkerName } from '../lib/traceUtils'
import type { TraceListItem } from './useTraceData'

/**
 * Spans per composition PROBE — one trace per read. A probe is complete for
 * the traces this path exists to judge (bookkeeping runs a handful of
 * spans) and a bounded slice of a fat one, which can only prove visibility.
 * The old shape — twenty traces per read, ten thousand spans — served a
 * turn's ~75KB spans as a multi-MB response that outgrew the transport's
 * ~16MiB delivery cap under load and never arrived, and with unknown
 * verdicts hiding, a read that never arrives is a row that never shows.
 */
const FETCH_SPAN_LIMIT = 80
/** Probes in flight at once — many rows can need one after a page change. */
const FETCH_CONCURRENCY = 4
/** Backoff before a failed or inconclusive probe is attempted again. */
export const FETCH_RETRY_MS = 10_000

/**
 * Why a listed row without a verdict is not being read right now: its probe
 * is in flight; its last probe found the trace still running (probe again
 * once the row settles); or its last probe failed or was inconclusive — a
 * truncated slice with no visible span — and waits out a backoff.
 */
export type CompositionReadState =
  | { kind: 'inflight' }
  | { kind: 'running' }
  | { kind: 'retry'; at: number }

/**
 * The listed rows whose composition must be read now: no verdict, and no
 * read state that says to wait. A row a previous read found running is
 * read again only once it has settled, so a long turn costs one read at its
 * start and one at its end, not one per list refresh. Exported for tests.
 */
export function selectCompositionReads(
  rows: readonly TraceListItem[],
  verdicts: ReadonlyMap<string, boolean>,
  reads: ReadonlyMap<string, CompositionReadState>,
  now: number,
): string[] {
  const out: string[] = []
  for (const row of rows) {
    if (verdicts.has(row.traceId)) continue
    const state = reads.get(row.traceId)
    if (
      state === undefined ||
      (state.kind === 'running' && row.status !== 'pending') ||
      (state.kind === 'retry' && now - state.at >= FETCH_RETRY_MS)
    ) {
      out.push(row.traceId)
    }
  }
  return out
}

/**
 * Forget read states for traces no longer listed, in-flight reads aside.
 * Verdicts are pruned the same way when a trace leaves both the list and
 * the feed (`reconcileTraceVisibility`), so a trace that pages back in is
 * read again — without this, a state left behind from its earlier visit
 * would keep it hidden for good. Mutates `reads`. Exported for tests.
 */
export function pruneCompositionReads(
  reads: Map<string, CompositionReadState>,
  listed: ReadonlySet<string>,
): void {
  for (const [traceId, state] of [...reads]) {
    if (!listed.has(traceId) && state.kind !== 'inflight') {
      reads.delete(traceId)
    }
  }
}

/**
 * Traces with work still in flight, judged from raw spans: a pending
 * snapshot (the engine's queue wrapper stays pending for a turn's whole
 * lifetime — see `followTurn.ts`). A closed span is never evidence of
 * liveness, however recent: a "just ended" window kept every hidden
 * bookkeeping trace visible for its duration and then hid it — the flicker
 * this module exists to avoid. Engines without live snapshots simply reveal
 * a hidden-rooted trace once its first visible span closes. Exported for
 * tests.
 */
export function pendingTraceIds(
  spans: readonly StoredSpan[],
): ReadonlySet<string> {
  const live = new Set<string>()
  for (const span of spans) {
    if (isPendingSpan(span)) live.add(span.trace_id)
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
 * Whether a listed row shows without waiting on the feed or a probe: it
 * is failed, still running, or rooted in a visible span. Everything else
 * is a hidden-rooted settled trace that hides until proven otherwise — the
 * "new traces" pill counts held rows with this, so it never promises rows
 * that would not appear. Exported for tests.
 */
export function showsWithoutProbe(
  row: TraceListItem,
  selection: SpanFilterSelection,
): boolean {
  return (
    row.status === 'error' ||
    row.status === 'pending' ||
    !isSpanBarHidden(rowRootFilterKeys(row), selection)
  )
}

/**
 * One reconcile pass: refresh `verdicts` from the bars currently in the
 * feed, seed visible-root verdicts from the rows themselves, drop entries
 * for traces gone from both the feed and the list, and return the rows
 * that survive: a proven-visible verdict, a failed trace, or a trace still
 * running (the row's `pending` status, or a pending snapshot in the feed
 * via `liveTraces`) — a running trace's negative verdict is a guess, its
 * visible spans may simply not have closed yet. Unknown verdicts hide.
 * Verdicts are monotone within a selection: `true` sticks, `false`
 * re-evaluates. Mutates `verdicts` — the hook owns the map across renders.
 * Exported for tests.
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
    (r) =>
      r.status === 'error' ||
      r.status === 'pending' ||
      verdicts.get(r.traceId) === true ||
      liveTraces.has(r.traceId),
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

/** Why a probed trace is still undecided after its probe. */
export type UndecidedReason = 'running' | 'truncated'

/**
 * Fold one composition probe into the verdicts: every requested trace's
 * verdict is "any of its bars survives the filter" — a trace the store no
 * longer has spans for counts as hidden (its detail would be empty). A
 * truncated response (span count at the limit) only trusts POSITIVE
 * verdicts: a visible span proves visibility, but "all hidden" might just
 * mean the visible spans were cut off. A `true` verdict already in the
 * cache is never downgraded — the probe raced a feed frame that proved
 * visibility after the snapshot was taken.
 *
 * A trace the probe finds still RUNNING (a pending snapshot among its
 * spans) gets no negative verdict either — its visible work may not have
 * closed yet. Every trace left undecided is returned with the reason, so
 * the caller can probe again at the right time. Exported for tests.
 */
export function mergeFetchedVerdicts(
  verdicts: Map<string, boolean>,
  requested: readonly string[],
  spans: readonly StoredSpan[],
  selection: SpanFilterSelection,
  spanLimit: number = FETCH_SPAN_LIMIT,
): ReadonlyMap<string, UndecidedReason> {
  const visible = new Set<string>()
  for (const bar of verdictBars(spans)) {
    if (bar.traceId && !isSpanBarHidden(bar, selection)) {
      visible.add(bar.traceId)
    }
  }
  const truncated = spans.length >= spanLimit
  const running = pendingTraceIds(spans)
  const undecided = new Map<string, UndecidedReason>()
  for (const traceId of requested) {
    if (visible.has(traceId)) verdicts.set(traceId, true)
    else if (running.has(traceId)) undecided.set(traceId, 'running')
    else if (truncated) undecided.set(traceId, 'truncated')
    else if (!verdicts.get(traceId)) verdicts.set(traceId, false)
  }
  return undecided
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
  // stale. `reads` remembers why a still-unverdicted trace is not being
  // read right now (see `CompositionReadState`).
  const cacheRef = useRef<{
    selection: SpanFilterSelection
    verdicts: Map<string, boolean>
    reads: Map<string, CompositionReadState>
  } | null>(null)

  // Bumped when a composition read lands verdicts, to re-run the memo.
  const [fetchTick, setFetchTick] = useState(0)

  // fetchTick isn't read in the body — it signals `cache.verdicts` grew.
  // The running set is derived from the feed at reconcile time (feed
  // frames arrive continuously while anything is live, so it stays fresh
  // without its own clock).
  // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  const visibleRows = useMemo(() => {
    let cache = cacheRef.current
    if (!cache || cache.selection !== selection) {
      cache = { selection, verdicts: new Map(), reads: new Map() }
      cacheRef.current = cache
    }
    return reconcileTraceVisibility(
      cache.verdicts,
      bars,
      rows,
      selection,
      pendingTraceIds(feedSpans),
    )
  }, [rows, bars, feedSpans, selection, fetchTick])

  // Composition probes for the rows neither source could judge: hidden
  // root, no feed coverage. Runs after the memo above, so feed verdicts
  // are already in the cache. Deliberately NO cancellation on dep churn —
  // live row appends must not orphan an in-flight probe (its trace would
  // stay in flight but never verdicted); a stale SELECTION is detected by
  // cache identity instead. A failed or inconclusive probe leaves its row
  // hidden until the feed decides or the probe is retried after
  // `FETCH_RETRY_MS` — the timer re-arms this effect so the retry does not
  // wait for the next row change.
  const [retryTick, setRetryTick] = useState(0)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(
    () => () => {
      if (retryTimerRef.current !== null) clearTimeout(retryTimerRef.current)
    },
    [],
  )
  // retryTick isn't read in the body — it re-runs the selection after a
  // retry backoff.
  // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  useEffect(() => {
    const cache = cacheRef.current
    if (!cache || cache.selection !== selection) return
    pruneCompositionReads(cache.reads, new Set(rows.map((r) => r.traceId)))
    const unknown = selectCompositionReads(
      rows,
      cache.verdicts,
      cache.reads,
      Date.now(),
    )
    if (unknown.length === 0) return
    for (const traceId of unknown)
      cache.reads.set(traceId, { kind: 'inflight' })
    const scheduleRetry = () => {
      if (retryTimerRef.current !== null) return
      retryTimerRef.current = setTimeout(() => {
        retryTimerRef.current = null
        setRetryTick((t) => t + 1)
      }, FETCH_RETRY_MS)
    }
    const probe = async (traceId: string) => {
      try {
        const res = await fetchTraceSpans({
          trace_id: traceId,
          search_all_spans: true,
          include_internal: true,
          sort_by: 'start_time',
          sort_order: 'asc',
          limit: FETCH_SPAN_LIMIT,
        })
        if (cacheRef.current !== cache) return
        const undecided = mergeFetchedVerdicts(
          cache.verdicts,
          [traceId],
          res.spans,
          selection,
        )
        const reason = undecided.get(traceId)
        if (reason === 'running') {
          cache.reads.set(traceId, { kind: 'running' })
        } else if (reason === 'truncated') {
          cache.reads.set(traceId, { kind: 'retry', at: Date.now() })
          scheduleRetry()
        } else {
          cache.reads.delete(traceId)
        }
        setFetchTick((t) => t + 1)
      } catch {
        // Traces unavailable (memory exporter off, transient error) —
        // the row stays hidden until the feed decides or the retry lands.
        if (cacheRef.current !== cache) return
        cache.reads.set(traceId, { kind: 'retry', at: Date.now() })
        scheduleRetry()
      }
    }
    // A small pool: probes are cheap and indexed by trace id, but a page
    // change can leave dozens of rows to judge at once.
    let next = 0
    const worker = async () => {
      while (next < unknown.length) {
        const traceId = unknown[next++]
        await probe(traceId)
      }
    }
    void Promise.all(
      Array.from(
        { length: Math.min(FETCH_CONCURRENCY, unknown.length) },
        worker,
      ),
    )
  }, [rows, selection, retryTick])

  return visibleRows
}
