import { describe, expect, it } from 'vitest'
import { type ChangeEntry, causeLabel, MAX_ENTRIES, recordChange, relativeAge, seedFromStatus } from './changes'
import type { ChangedEvent } from './events'

function event(path: string, over: Partial<ChangedEvent> = {}): ChangedEvent {
  return {
    path,
    cause: 'shell::fs::write',
    kind: 'modified',
    added: 3,
    removed: 1,
    patch: '@@ -1 +1 @@',
    truncated: false,
    root: '/repo',
    ...over,
  }
}

describe('recordChange', () => {
  it('puts the newest change first', () => {
    let log: ChangeEntry[] = []
    log = recordChange(log, event('a.ts'), 1_000)
    log = recordChange(log, event('b.ts'), 2_000)
    expect(log.map((e) => e.path)).toEqual(['b.ts', 'a.ts'])
  })

  it('collapses repeats of one path and counts them', () => {
    let log: ChangeEntry[] = []
    log = recordChange(log, event('a.ts', { added: 1 }), 1_000)
    log = recordChange(log, event('b.ts'), 2_000)
    log = recordChange(log, event('a.ts', { added: 9 }), 3_000)
    expect(log).toHaveLength(2)
    expect(log[0]).toMatchObject({ path: 'a.ts', added: 9, count: 2, at: 3_000 })
    expect(log[1].path).toBe('b.ts')
  })

  it('is bounded', () => {
    let log: ChangeEntry[] = []
    for (let i = 0; i < MAX_ENTRIES + 10; i++) log = recordChange(log, event(`f${i}.ts`), i)
    expect(log).toHaveLength(MAX_ENTRIES)
    expect(log[0].path).toBe(`f${MAX_ENTRIES + 9}.ts`)
  })
})

describe('seedFromStatus', () => {
  it('adds working-tree paths with no timestamp', () => {
    const log = seedFromStatus([], [{ path: 'a.ts', index: ' ', worktree: 'M' }])
    expect(log).toEqual([expect.objectContaining({ path: 'a.ts', kind: 'modified', at: null, count: 0, cause: 'git' })])
  })

  it('never overwrites a path that has a real event', () => {
    const live = recordChange([], event('a.ts'), 1_000)
    const log = seedFromStatus(live, [{ path: 'a.ts', index: ' ', worktree: 'M' }])
    expect(log).toHaveLength(1)
    expect(log[0].at).toBe(1_000)
  })

  it('maps status codes to change kinds', () => {
    const rows = seedFromStatus(
      [],
      [
        { path: 'new.ts', index: '?', worktree: '?' },
        { path: 'gone.ts', index: ' ', worktree: 'D' },
        { path: 'moved.ts', index: 'R', worktree: ' ' },
      ],
    )
    expect(rows.map((r) => r.kind)).toEqual(['created', 'deleted', 'moved'])
  })
})

describe('causeLabel', () => {
  it('names the surface that performed the write', () => {
    expect(causeLabel('shell::fs::write')).toBe('shell')
    expect(causeLabel('editor::save')).toBe('editor')
    expect(causeLabel('git')).toBe('working tree')
    expect(causeLabel('')).toBe('unknown')
  })
})

describe('relativeAge', () => {
  it('reads as recency', () => {
    expect(relativeAge(null, 10_000)).toBe('earlier')
    expect(relativeAge(9_000, 10_000)).toBe('now')
    expect(relativeAge(0, 30_000)).toBe('30s')
    expect(relativeAge(0, 120_000)).toBe('2m')
    expect(relativeAge(0, 7_200_000)).toBe('2h')
  })
})

describe('session provenance', () => {
  it('carries the session and turn from the event', () => {
    const log = recordChange([], event('a.ts', { session_id: 's_abc', turn_id: 't_1' }), 1_000)
    expect(log[0]).toMatchObject({ sessionId: 's_abc', turnId: 't_1' })
  })

  it('leaves them unset for a write made outside a turn', () => {
    const log = recordChange([], event('a.ts'), 1_000)
    expect(log[0].sessionId).toBeUndefined()
  })

  it('keeps the patch, which is what the diff view reads', () => {
    const log = recordChange([], event('a.ts', { patch: '@@ -1 +1 @@\n-old\n+new' }), 1_000)
    expect(log[0].patch).toContain('+new')
  })
})
