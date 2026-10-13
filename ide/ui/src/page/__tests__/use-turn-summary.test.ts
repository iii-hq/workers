import { describe, expect, it } from 'vitest'
import type { ShellReviewFileSummary } from '../review-summary-store'
import type { SessionTurn } from '../turns'
import { knownRow, mergeSummaryRows, totalsHold } from '../use-turn-summary'

const row = (path: string, add: number | null = null): ShellReviewFileSummary => ({
  path,
  state: add === null ? 'pending' : 'ready',
  add,
  del: add === null ? null : 0,
})

describe('mergeSummaryRows', () => {
  it('keeps the list when nothing is news, and every untouched row when something is', () => {
    const previous = [row('a.ts', 1), row('b.ts')]
    expect(mergeSummaryRows(previous, new Map([['a.ts', row('a.ts', 1)]]))).toBe(previous)
    const next = mergeSummaryRows(previous, new Map([['b.ts', row('b.ts', 2)]]))
    expect(next).not.toBe(previous)
    expect(next[0]).toBe(previous[0])
    expect(next[1]).toEqual(row('b.ts', 2))
  })
})

describe('knownRow', () => {
  const record: SessionTurn = { turn_id: 't', started_at: 1, files: [] }

  it('hands held totals back under the path relative to the current root', () => {
    const known = new Map([['/r/sub/a.ts', { record, row: row('sub/a.ts', 3) }]])
    expect(knownRow(known, record, '/r', 'sub/a.ts')).toBe(known.get('/r/sub/a.ts')?.row)
    // Re-rooted to the child: same file, same totals, its own path.
    expect(knownRow(known, record, '/r/sub', 'a.ts')).toEqual(row('a.ts', 3))
    // Totals from another record of the turn are read again.
    expect(knownRow(known, { ...record }, '/r', 'sub/a.ts')).toBeNull()
    expect(knownRow(known, record, '/r', 'b.ts')).toBeNull()
  })
})

describe('totalsHold', () => {
  const turn = (file: Partial<SessionTurn['files'][number]>): SessionTurn => ({
    turn_id: 't',
    started_at: 1,
    files: [{ path: '/r/a.ts', kind: 'modified', cause: 'coder::update-file', first_seen: 1, last_seen: 1, ...file }],
  })

  it('holds against a kept pre-image, and never against the last commit', () => {
    expect(totalsHold(turn({ before: { content: 'v1' } }), '/r', 'a.ts')).toBe(true)
    expect(totalsHold(turn({ kind: 'created' }), '/r', 'a.ts')).toBe(true)
    expect(totalsHold(turn({}), '/r', 'untouched.ts')).toBe(true)
    expect(totalsHold(turn({}), '/r', 'a.ts')).toBe(false)
    expect(totalsHold(turn({ before: { truncated: true } }), '/r', 'a.ts')).toBe(false)
  })
})
