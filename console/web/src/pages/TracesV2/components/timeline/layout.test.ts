import { describe, expect, it } from 'vitest'
import {
  computeAssignments,
  type SpanAssignment,
  type TimelineSpan,
} from './layout'

/** An (effectively) instant trace: a few ms of root span. */
function instant(id: string, startTime: number, durationMs = 5): TimelineSpan {
  return { id, startTime, endTime: startTime + durationMs }
}

const NONE: ReadonlyMap<string, SpanAssignment> = new Map()

function lanesOf(
  assignments: ReadonlyMap<string, SpanAssignment>,
  ids: string[],
): Array<{ type: string; lane: number }> {
  return ids.map((id) => {
    const a = assignments.get(id)
    if (!a) throw new Error(`no assignment for ${id}`)
    return { type: a.type, lane: a.lane }
  })
}

describe('computeAssignments (visual clearance)', () => {
  it('spreads near-simultaneous instants across the threads', () => {
    // Four ms-long traces landing 300ms apart: temporally disjoint, but at
    // min bar width they overlap on screen — each should take a new lane.
    const spans = [0, 300, 600, 900].map((t, i) => instant(`s${i}`, t))
    const out = computeAssignments(spans, NONE, 4, 1_200)
    expect(lanesOf(out, ['s0', 's1', 's2', 's3'])).toEqual([
      { type: 'bar', lane: 0 },
      { type: 'bar', lane: 1 },
      { type: 'bar', lane: 2 },
      { type: 'bar', lane: 3 },
    ])
  })

  it('keeps sparse traffic hugging the axis', () => {
    // Gaps larger than the clearance: every span comes back to lane 0.
    const spans = [0, 5_000, 10_000].map((t, i) => instant(`s${i}`, t))
    const out = computeAssignments(spans, NONE, 4, 1_200)
    expect(lanesOf(out, ['s0', 's1', 's2'])).toEqual([
      { type: 'bar', lane: 0 },
      { type: 'bar', lane: 0 },
      { type: 'bar', lane: 0 },
    ])
  })

  it('reverts to first-free-lane packing when clearance is 0', () => {
    const spans = [0, 300, 600].map((t, i) => instant(`s${i}`, t))
    const out = computeAssignments(spans, NONE, 4, 0)
    expect(lanesOf(out, ['s0', 's1', 's2'])).toEqual([
      { type: 'bar', lane: 0 },
      { type: 'bar', lane: 0 },
      { type: 'bar', lane: 0 },
    ])
  })

  it('overflows onto the most-cleared lane once every thread is crowded', () => {
    // Five instants inside one clearance window and only 4 lanes: the fifth
    // must still be a BAR (no temporal overlap anywhere) on the lane whose
    // occupant cleared first — pixel overlap is the least-bad option.
    const spans = [0, 200, 400, 600, 800].map((t, i) => instant(`s${i}`, t))
    const out = computeAssignments(spans, NONE, 4, 1_200)
    expect(out.get('s4')).toEqual({ spanId: 's4', type: 'bar', lane: 0 })
  })

  it('never lane-shares spans that truly overlap in time', () => {
    // Two long-running (open-ended) spans plus a third: all three overlap,
    // so with 2 lanes the third becomes a chip on the longest-running lane.
    const spans: TimelineSpan[] = [
      { id: 'a', startTime: 0, endTime: null },
      { id: 'b', startTime: 100, endTime: null },
      { id: 'c', startTime: 200, endTime: 300 },
    ]
    const out = computeAssignments(spans, NONE, 2, 1_200)
    expect(out.get('a')?.type).toBe('bar')
    expect(out.get('b')?.type).toBe('bar')
    expect(out.get('a')?.lane).not.toBe(out.get('b')?.lane)
    expect(out.get('c')?.type).toBe('chip')
  })

  it('keeps prior placements sticky across recomputes', () => {
    const spans = [0, 300, 600].map((t, i) => instant(`s${i}`, t))
    const first = computeAssignments(spans, NONE, 4, 1_200)
    // Feed the output back with a new span appended: old ids keep their
    // lanes, the newcomer is placed around them.
    const more = [...spans, instant('s3', 650)]
    const second = computeAssignments(more, first, 4, 1_200)
    for (const id of ['s0', 's1', 's2']) {
      expect(second.get(id)).toEqual(first.get(id))
    }
    expect(second.get('s3')).toEqual({ spanId: 's3', type: 'bar', lane: 3 })
  })
})
