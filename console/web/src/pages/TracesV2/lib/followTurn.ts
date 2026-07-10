/**
 * Follow-the-turn selection: which trace the masthead should auto-open to
 * watch the chat's current work.
 *
 * Every span a harness turn step spawns carries the turn's identity as
 * baggage attributes (`harness/src/functions/turn.rs`): `iii.session.id`
 * (the chat session), `iii.message.id` (the turn) and `iii.tag.kind` —
 * `harness.turn` for a top-level user turn, `harness.subagent` for spawned
 * children. Only `harness.turn` spans of the active chat session qualify,
 * so sub-agent activity and other sessions' turns never steal the view.
 *
 * Delivery reality (verified against a live engine): the tagged spans are
 * WORKER spans, exported through a batch processor — they reach the store
 * and the all-spans stream only when they CLOSE. A turn step is therefore
 * never visible as a `pending` tagged span; instead its short-lived
 * children (`execute session::set-status`, 0ms, closes milliseconds into
 * the step) announce the turn's trace within ~half a second of the send.
 * The engine's live `pending` snapshots exist only for its OWN spans
 * (e.g. `fn_queue default`), which carry no session identity — useful as
 * a "this trace is still running" signal, never for attribution.
 *
 * Hence the model: `evaluateFollow` keeps the set of turn traces already
 * seen for the session and reports a NEW tagged trace as the one to open.
 * On the first evaluation (toggle-on / session switch) the current traces
 * are baselined without opening — except a still-running one (any pending
 * span in the trace), so enabling mid-turn jumps to the live work.
 */

import type { StoredSpan } from '../api/traces'
import { normalizeSpanAttributes } from './traceListItem'
import { isPendingSpan, toMs } from './traceTransform'

/** `iii.tag.kind` stamped on every span of a top-level turn step. */
export const TURN_KIND_ATTR = 'iii.tag.kind'
export const TURN_KIND_TOP_LEVEL = 'harness.turn'
export const SESSION_ID_ATTR = 'iii.session.id'

/** Traces with at least one `harness.turn` span of `sessionId`, as a
 *  trace_id → newest qualifying span start (ms) map. */
export function turnTracesFor(
  spans: readonly StoredSpan[],
  sessionId: string,
): Map<string, number> {
  const traces = new Map<string, number>()
  for (const span of spans) {
    const attrs = normalizeSpanAttributes(span.attributes)
    if (attrs[TURN_KIND_ATTR] !== TURN_KIND_TOP_LEVEL) continue
    if (attrs[SESSION_ID_ATTR] !== sessionId) continue
    const start = toMs(span.start_time_unix_nano)
    const known = traces.get(span.trace_id)
    if (known === undefined || start > known) traces.set(span.trace_id, start)
  }
  return traces
}

/** Whether any span of the trace is still live (the engine's `live_spans`
 *  pending mirror — absent on engines without it, which just means a
 *  mid-turn toggle-on won't jump to the already-running work). */
export function traceHasPendingSpan(
  spans: readonly StoredSpan[],
  traceId: string,
): boolean {
  return spans.some((s) => s.trace_id === traceId && isPendingSpan(s))
}

/** Follow memory for one (enabled, session) run: which turn traces have
 *  already been considered, so each one opens at most once. */
export interface FollowState {
  sessionId: string
  seenTraceIds: Set<string>
}

export interface FollowDecision {
  state: FollowState
  /** Trace to open now, or `null` when the view should stay put. */
  openTraceId: string | null
}

/**
 * One follow evaluation over the current all-spans feed. Pass the previous
 * decision's `state` (or `null` when following just started); the session
 * changing rebaselines automatically.
 *
 * - First evaluation for a session: baseline every current turn trace
 *   without opening, EXCEPT the newest one if it is still running (any
 *   pending span) — enabling mid-turn should show the live work.
 * - Later evaluations: a turn trace not yet seen opens (newest first) and
 *   joins the set. Already-seen traces never re-open, so closing or
 *   navigating away mid-turn is respected until the next turn.
 */
export function evaluateFollow(
  state: FollowState | null,
  spans: readonly StoredSpan[],
  sessionId: string,
): FollowDecision {
  const traces = turnTracesFor(spans, sessionId)

  let newest: string | null = null
  let newestStart = Number.NEGATIVE_INFINITY
  for (const [traceId, start] of traces) {
    if (start > newestStart) {
      newest = traceId
      newestStart = start
    }
  }

  if (!state || state.sessionId !== sessionId) {
    const next: FollowState = {
      sessionId,
      seenTraceIds: new Set(traces.keys()),
    }
    const openLive =
      newest !== null && traceHasPendingSpan(spans, newest) ? newest : null
    return { state: next, openTraceId: openLive }
  }

  if (newest === null || state.seenTraceIds.has(newest)) {
    return { state, openTraceId: null }
  }
  // Adopt every currently-visible trace, not just the winner: an older
  // unseen trace must not pop open on a later frame after retention prunes
  // the newer one.
  for (const traceId of traces.keys()) state.seenTraceIds.add(traceId)
  return { state, openTraceId: newest }
}
