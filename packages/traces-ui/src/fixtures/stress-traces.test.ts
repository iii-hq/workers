import { describe, expect, it } from 'vitest'
import { toWaterfallData } from '../lib/traceTransform'
import {
  generateStressTrace,
  STRESS_T0,
  snapshotAt,
  summarizeStressTrace,
} from './stress-traces'

describe('generateStressTrace', () => {
  it('yields exactly the requested span count, deterministically', () => {
    const a = generateStressTrace({ spanCount: 2000, seed: 3 })
    const b = generateStressTrace({ spanCount: 2000, seed: 3 })
    expect(a).toHaveLength(2000)
    expect(JSON.stringify(a)).toBe(JSON.stringify(b))
    expect(
      JSON.stringify(generateStressTrace({ spanCount: 2000, seed: 4 })),
    ).not.toBe(JSON.stringify(a))
  })

  it('builds a well-formed tree: one root, parents exist and contain their children', () => {
    const spans = generateStressTrace({ spanCount: 1500, seed: 11 })
    const byId = new Map(spans.map((s) => [s.span_id, s]))
    const roots = spans.filter((s) => !s.parent_span_id)
    expect(roots).toHaveLength(1)
    expect(roots[0].name).toBe('harness::turn step')
    for (const s of spans) {
      if (!s.parent_span_id) continue
      const parent = byId.get(s.parent_span_id)
      expect(parent, `parent of ${s.span_id}`).toBeDefined()
      if (!parent) continue
      expect(s.start_time_unix_nano).toBeGreaterThanOrEqual(
        parent.start_time_unix_nano,
      )
      expect(s.end_time_unix_nano).toBeLessThanOrEqual(
        parent.end_time_unix_nano,
      )
      expect(s.end_time_unix_nano).toBeGreaterThan(s.start_time_unix_nano)
    }
    const depths = new Set<number>()
    for (const s of spans) {
      let d = 0
      let cur: typeof s | undefined = s
      while (cur?.parent_span_id) {
        cur = byId.get(cur.parent_span_id)
        d++
      }
      depths.add(d)
    }
    expect(Math.max(...depths)).toBeGreaterThanOrEqual(3)
    expect(
      new Set(spans.map((s) => s.service_name)).size,
    ).toBeGreaterThanOrEqual(6)
    expect(spans.filter((s) => s.status === 'ERROR').length).toBeGreaterThan(0)
  })

  it('feeds the production transform without loss', () => {
    const spans = generateStressTrace({ spanCount: 800, seed: 5 })
    const wf = toWaterfallData(spans, spans[0].trace_id)
    expect(wf?.spans).toHaveLength(800)
    expect(wf?.total_duration_ms).toBeGreaterThan(29_000)
  })

  it('snapshots the trace mid-flight with the open spans pending', () => {
    const spans = generateStressTrace({
      spanCount: 600,
      seed: 9,
      durationMs: 10_000,
    })
    const half = snapshotAt(spans, STRESS_T0 + 5_000)
    expect(half.length).toBeGreaterThan(50)
    expect(half.length).toBeLessThan(600)
    const pending = half.filter((s) => s.pending)
    expect(pending.length).toBeGreaterThan(0)
    for (const p of pending) expect(p.end_time_unix_nano).toBe(0)
    expect(half[0].pending).toBe(true) // the root runs until the end
    expect(snapshotAt(spans, STRESS_T0 + 10_001)).toHaveLength(600)
    expect(summarizeStressTrace(half).status).toBe('pending')
    expect(summarizeStressTrace(spans).span_count).toBe(600)
  })
})
