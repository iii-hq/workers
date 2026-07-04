import { describe, expect, it } from 'vitest'
import type { CommEvent } from '@/types/iii-agent-event'
import { buildLanes, mergeEvents, resolveRootId } from './timeline'

function ev(seq: number, over: Partial<CommEvent> = {}): CommEvent {
  return {
    seq,
    at: 1000 + seq,
    root_session_id: 's_root',
    kind: 'spawn',
    from: { session_id: 's_root' },
    to: { session_id: `s_c${seq}` },
    ...over,
  }
}

describe('mergeEvents', () => {
  it('dedupes by seq and sorts', () => {
    const merged = mergeEvents(
      [ev(2), ev(1)],
      [ev(2, { summary: 'dup' }), ev(3)],
    )
    expect(merged.map((e) => e.seq)).toEqual([1, 2, 3])
    expect(merged[1]?.summary).toBe('dup') // incoming wins
  })

  it('keeps seq-0 live events, ordered by at', () => {
    const a = ev(0, { at: 5000 })
    const b = ev(0, { at: 6000 })
    const merged = mergeEvents([ev(1, { at: 4000 }), a], [b])
    expect(merged.map((e) => e.at)).toEqual([4000, 5000, 6000])
  })
})

describe('buildLanes', () => {
  it('root first, then order of appearance including react children', () => {
    const lanes = buildLanes('s_root', [
      ev(1, { to: { session_id: 's_a' } }),
      ev(2, {
        kind: 'trigger_fire',
        from: undefined,
        to: undefined,
        trigger: { action: 'react', child_session_id: 's_b' },
      }),
      ev(3, { from: { session_id: 's_a' }, to: { session_id: 's_root' } }),
    ])
    expect(lanes).toEqual(['s_root', 's_a', 's_b'])
  })
})

describe('resolveRootId', () => {
  it('walks parents and survives cycles', () => {
    const parents: Record<string, string | null> = {
      s_c: 's_b',
      s_b: 's_a',
      s_a: null,
    }
    expect(resolveRootId('s_c', (id) => parents[id])).toBe('s_a')
    const cyclic: Record<string, string> = { x: 'y', y: 'x' }
    expect(resolveRootId('x', (id) => cyclic[id])).toBe('x')
  })
})
