import { describe, expect, it } from 'vitest'
import {
  baselineFor,
  relativeToRoot,
  reviewEntriesFromTurn,
  type SessionTurn,
  turnLabel,
} from '../turns'

describe('relativeToRoot', () => {
  it('relativizes paths under the root and rejects the rest', () => {
    expect(relativeToRoot('/r/a/b.ts', '/r')).toBe('a/b.ts')
    expect(relativeToRoot('/r/a/b.ts', '/r/')).toBe('a/b.ts')
    expect(relativeToRoot('/r', '/r')).toBeNull()
    expect(relativeToRoot('/rx/a', '/r')).toBeNull()
    expect(relativeToRoot('/other/a', '/r')).toBeNull()
  })
})

describe('baselineFor', () => {
  it('maps the stored pre-image onto the review baseline contract', () => {
    expect(baselineFor({ content: 'old' })).toBe('old')
    expect(baselineFor({ missing: true })).toBe('')
    expect(baselineFor({ content: 'part', truncated: true })).toBeNull()
    expect(baselineFor({ binary: true, revision: 'sha256:x' })).toBeNull()
    expect(baselineFor(null)).toBeNull()
    expect(baselineFor(undefined)).toBeNull()
    expect(baselineFor({})).toBeNull()
  })
})

describe('reviewEntriesFromTurn', () => {
  const turn: SessionTurn = {
    turn_id: 't1',
    started_at: 1,
    ended_at: 2,
    files: [
      {
        path: '/r/src/a.rs',
        kind: 'modified',
        cause: 'shell::fs::write',
        first_seen: 1,
        last_seen: 1,
        before: { content: 'fn a() {}\n' },
      },
      {
        path: '/r/src/new.rs',
        kind: 'created',
        cause: 'coder::create-file',
        first_seen: 1,
        last_seen: 1,
        before: { missing: true },
      },
      {
        path: '/r/docs/gone.md',
        kind: 'deleted',
        cause: 'shell::fs::rm',
        first_seen: 1,
        last_seen: 1,
        before: { content: '# gone\n' },
      },
      {
        path: '/r/src/b.rs',
        kind: 'moved',
        cause: 'shell::fs::mv',
        first_seen: 1,
        last_seen: 1,
        from: '/r/src/old-b.rs',
      },
      {
        path: '/elsewhere/x.txt',
        kind: 'modified',
        cause: 'shell::fs::write',
        first_seen: 1,
        last_seen: 1,
      },
    ],
  }

  it('gives an observed creation an empty baseline', () => {
    const observed: SessionTurn = {
      turn_id: 't2',
      started_at: 1,
      ended_at: 2,
      files: [
        {
          path: '/r/gen/out.txt',
          kind: 'created',
          cause: 'observed',
          first_seen: 1,
          last_seen: 1,
        },
        {
          path: '/r/gen/tweaked.txt',
          kind: 'modified',
          cause: 'observed',
          first_seen: 1,
          last_seen: 1,
        },
      ],
    }
    const { entries } = reviewEntriesFromTurn(observed, '/r')
    expect(entries.get('gen/out.txt')).toMatchObject({
      change: { status: 'added' },
      baseline: '',
    })
    expect(entries.get('gen/tweaked.txt')).toMatchObject({
      change: { status: 'modified' },
      baseline: null,
    })
  })

  it('builds review entries under the root and counts the rest', () => {
    const { entries, outside } = reviewEntriesFromTurn(turn, '/r')
    expect(outside).toBe(1)
    expect([...entries.keys()]).toEqual([
      'src/a.rs',
      'src/new.rs',
      'docs/gone.md',
      'src/b.rs',
    ])
    expect(entries.get('src/a.rs')).toMatchObject({
      change: { status: 'modified', staged: false },
      baseline: 'fn a() {}\n',
    })
    expect(entries.get('src/new.rs')).toMatchObject({
      change: { status: 'added' },
      baseline: '',
    })
    expect(entries.get('docs/gone.md')).toMatchObject({
      change: { status: 'deleted' },
      baseline: '# gone\n',
    })
    expect(entries.get('src/b.rs')).toMatchObject({
      change: { status: 'renamed', from: 'src/old-b.rs' },
      baseline: null,
    })
  })
})

describe('turnLabel', () => {
  it('shows a time and a file count', () => {
    expect(turnLabel({ started_at: Date.UTC(2026, 7, 20, 12, 0), file_count: 1 })).toMatch(/1 file$/)
    expect(turnLabel({ started_at: Date.UTC(2026, 7, 20, 12, 0), file_count: 3 })).toMatch(/3 files$/)
    expect(turnLabel({ started_at: Number.NaN, file_count: 0 })).toBe('turn · 0 files')
  })
})
