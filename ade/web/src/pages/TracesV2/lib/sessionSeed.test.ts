import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import { seedSessions, seedTrace } from './sessionSeed'

const T0_MS = 1_783_628_073_000
const T1_MS = 1_783_628_075_000

/** A closed span of a top-level turn of `session`. */
function turnSpan(
  session: string,
  traceId: string,
  startMs = T0_MS,
): StoredSpan {
  return {
    trace_id: traceId,
    span_id: `s-${traceId}`,
    name: 'execute session::set-status',
    start_time_unix_nano: startMs * 1_000_000,
    end_time_unix_nano: (startMs + 4) * 1_000_000,
    status: 'ok',
    attributes: [
      ['iii.tag.kind', 'harness.turn'],
      ['iii.session.id', session],
    ],
    events: [],
    links: [],
  }
}

const row = (traceId: string, session?: string) => ({
  traceId,
  traceTags: session ? { 'iii.session.id': session } : undefined,
})

describe('seedSessions', () => {
  it('is the active chat when nothing runs', () => {
    expect(seedSessions('a', [{ id: 'a', status: 'done' }])).toEqual(['a'])
  })

  it('puts the one running chat first when the active one is idle', () => {
    expect(
      seedSessions('a', [
        { id: 'a', status: 'done' },
        { id: 'b', status: 'working' },
      ]),
    ).toEqual(['b', 'a'])
  })

  it('keeps the active chat when it is the one running', () => {
    expect(
      seedSessions('a', [
        { id: 'a', status: 'working' },
        { id: 'b', status: 'working' },
      ]),
    ).toEqual(['a'])
  })

  it('does not guess between several running chats', () => {
    expect(
      seedSessions('a', [
        { id: 'a', status: 'idle' },
        { id: 'b', status: 'working' },
        { id: 'c', status: 'working' },
      ]),
    ).toEqual(['a'])
  })

  it('ignores sub-agent sessions and drafts', () => {
    expect(
      seedSessions('a', [
        { id: 'a', status: 'idle' },
        { id: 'sub', status: 'working', parentId: 'a' },
        { id: 'd', status: 'working', draft: true },
      ]),
    ).toEqual(['a'])
  })

  it('has nothing to seed from with no chat and nothing running', () => {
    expect(seedSessions(null, [])).toEqual([])
  })
})

describe('seedTrace', () => {
  it('opens the newest live turn of the first session that has one', () => {
    const spans = [
      turnSpan('a', 't-old'),
      turnSpan('a', 't-new', T1_MS),
      turnSpan('b', 't-b'),
    ]
    expect(seedTrace(['a', 'b'], spans, [])).toBe('t-new')
    expect(seedTrace(['b', 'a'], spans, [])).toBe('t-b')
  })

  it('falls back to the list for a turn older than the live feed', () => {
    const rows = [row('t-other', 'z'), row('t-a', 'a'), row('t-a-older', 'a')]
    expect(seedTrace(['a'], [], rows)).toBe('t-a')
  })

  it('never takes an untagged row of the unscoped list', () => {
    expect(seedTrace(['a'], [], [row('t-untagged')])).toBeNull()
  })

  it('moves on to the next session when the first has nothing', () => {
    expect(seedTrace(['b', 'a'], [], [row('t-a', 'a')])).toBe('t-a')
  })
})
