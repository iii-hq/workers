import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import {
  evaluateFollow,
  traceHasPendingSpan,
  turnTracesFor,
} from './followTurn'

const SESSION = 'console-abc'

/** Realistic epoch instants (ms), converted to nanos in the fixtures so
 *  `toMs`'s nano-vs-ms heuristic takes the nano branch like production data. */
const T0_MS = 1_783_628_073_000
const T1_MS = 1_783_628_075_000

/**
 * A CLOSED span tagged with the turn baggage — the shape that actually
 * arrives on the feed (worker spans export on close; a tagged span is
 * never pending).
 */
function turnSpan(overrides: Partial<StoredSpan> = {}): StoredSpan {
  return {
    trace_id: 't-1',
    span_id: 's-1',
    name: 'execute session::set-status',
    start_time_unix_nano: T0_MS * 1_000_000,
    end_time_unix_nano: (T0_MS + 4) * 1_000_000,
    status: 'ok',
    attributes: [
      ['iii.tag.kind', 'harness.turn'],
      ['iii.session.id', SESSION],
      ['iii.message.id', 'turn-1'],
    ],
    events: [],
    links: [],
    ...overrides,
  }
}

/** The engine's live mirror of its own queue span: pending but UNTAGGED. */
function pendingQueueSpan(traceId: string): StoredSpan {
  return turnSpan({
    trace_id: traceId,
    span_id: `pending-${traceId}`,
    name: 'fn_queue default',
    end_time_unix_nano: 0,
    pending: true,
    status: 'unset',
    attributes: [['iii.tag.kind', 'queue.process']],
  })
}

describe('turnTracesFor', () => {
  it('collects CLOSED tagged spans — worker spans only ever arrive closed', () => {
    expect(turnTracesFor([turnSpan()], SESSION).has('t-1')).toBe(true)
  })

  it('ignores sub-agent steps — only the user turn may steal the view', () => {
    const sub = turnSpan({
      attributes: [
        ['iii.tag.kind', 'harness.subagent'],
        ['iii.session.id', SESSION],
      ],
    })
    expect(turnTracesFor([sub], SESSION).size).toBe(0)
  })

  it("ignores other sessions' turns and untagged spans", () => {
    expect(turnTracesFor([turnSpan()], 'console-other').size).toBe(0)
    const plain = turnSpan({ attributes: [['function_id', 'engine::echo']] })
    expect(turnTracesFor([plain], SESSION).size).toBe(0)
  })

  it('keeps the newest qualifying start per trace', () => {
    const traces = turnTracesFor(
      [
        turnSpan(),
        turnSpan({
          span_id: 's-2',
          start_time_unix_nano: T1_MS * 1_000_000,
        }),
      ],
      SESSION,
    )
    expect(traces.get('t-1')).toBe(T1_MS)
  })
})

describe('traceHasPendingSpan', () => {
  it('detects a live trace through ANY pending span, tags irrelevant', () => {
    const spans = [turnSpan(), pendingQueueSpan('t-1')]
    expect(traceHasPendingSpan(spans, 't-1')).toBe(true)
    expect(traceHasPendingSpan([turnSpan()], 't-1')).toBe(false)
  })
})

describe('evaluateFollow', () => {
  it('baselines pre-existing finished turns without opening them', () => {
    const { state, openTraceId } = evaluateFollow(null, [turnSpan()], SESSION)
    expect(openTraceId).toBeNull()
    expect(state.seenTraceIds.has('t-1')).toBe(true)
  })

  it('opens a still-running turn on the first evaluation (mid-turn toggle-on)', () => {
    const spans = [turnSpan(), pendingQueueSpan('t-1')]
    const { openTraceId } = evaluateFollow(null, spans, SESSION)
    expect(openTraceId).toBe('t-1')
  })

  it('opens a turn trace that appears AFTER the baseline', () => {
    const first = evaluateFollow(null, [], SESSION)
    expect(first.openTraceId).toBeNull()

    const next = evaluateFollow(first.state, [turnSpan()], SESSION)
    expect(next.openTraceId).toBe('t-1')
  })

  it('opens each turn trace at most once (closing mid-turn is respected)', () => {
    const first = evaluateFollow(null, [], SESSION)
    const opened = evaluateFollow(first.state, [turnSpan()], SESSION)
    expect(opened.openTraceId).toBe('t-1')

    // Later frames of the same turn (more tagged spans, same trace).
    const again = evaluateFollow(
      opened.state,
      [turnSpan(), turnSpan({ span_id: 's-2' })],
      SESSION,
    )
    expect(again.openTraceId).toBeNull()
  })

  it('picks the newest trace when several appear, and adopts the rest', () => {
    const first = evaluateFollow(null, [], SESSION)
    const older = turnSpan()
    const newer = turnSpan({
      trace_id: 't-2',
      span_id: 's-2',
      start_time_unix_nano: T1_MS * 1_000_000,
    })
    const opened = evaluateFollow(first.state, [older, newer], SESSION)
    expect(opened.openTraceId).toBe('t-2')

    // The older sibling was adopted too: it must not pop open later once
    // retention prunes t-2 out of the feed.
    const later = evaluateFollow(opened.state, [older], SESSION)
    expect(later.openTraceId).toBeNull()
  })

  it('baselines a DELAYED older trace arriving after the newest was seen', () => {
    const first = evaluateFollow(null, [], SESSION)
    const newer = turnSpan({
      trace_id: 't-2',
      span_id: 's-2',
      start_time_unix_nano: T1_MS * 1_000_000,
    })
    const opened = evaluateFollow(first.state, [newer], SESSION)
    expect(opened.openTraceId).toBe('t-2')

    // An older trace's spans arrive late, on a frame whose newest (t-2) is
    // already seen — the quiet early-return must still adopt it…
    const older = turnSpan()
    const quiet = evaluateFollow(opened.state, [newer, older], SESSION)
    expect(quiet.openTraceId).toBeNull()

    // …so it cannot pop open once retention prunes t-2 and the stale
    // t-1 becomes the newest trace on the feed.
    const pruned = evaluateFollow(quiet.state, [older], SESSION)
    expect(pruned.openTraceId).toBeNull()
  })

  it('rebaselines when the active session changes', () => {
    const a = evaluateFollow(null, [turnSpan()], SESSION)
    const otherTurn = turnSpan({
      trace_id: 't-9',
      span_id: 's-9',
      attributes: [
        ['iii.tag.kind', 'harness.turn'],
        ['iii.session.id', 'console-other'],
      ],
    })
    // Switching sessions: the other session's finished turn is baseline,
    // not an event to react to.
    const b = evaluateFollow(a.state, [otherTurn], 'console-other')
    expect(b.openTraceId).toBeNull()
    expect(b.state.sessionId).toBe('console-other')
    expect(b.state.seenTraceIds.has('t-9')).toBe(true)
  })
})
