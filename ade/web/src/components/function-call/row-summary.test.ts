import { describe, expect, it } from 'vitest'
import { errorSubtitle, formatRowDuration, resultSummary } from './row-summary'

describe('resultSummary', () => {
  it('counts a bare list and a single array field', () => {
    expect(resultSummary([1, 2, 3, 4])).toBe('4 results')
    expect(resultSummary({ files: [{}, {}, {}] })).toBe('3 files')
    expect(resultSummary({ matches: [{}] })).toBe('1 match')
    expect(resultSummary({ entries: [{}, {}] })).toBe('2 entries')
  })

  it('spells irregular plurals', () => {
    expect(resultSummary({ children: [] })).toBe('0 children')
    expect(resultSummary({ children: [{}] })).toBe('1 child')
    expect(resultSummary({ children: [{}, {}] })).toBe('2 children')
    expect(resultSummary({ people: [{}, {}] })).toBe('2 people')
  })

  it('says the state of a status report instead of counting its side list', () => {
    // harness::status on a child session: its own `children` is empty,
    // which read as "0 childrens" beside a sub-agent that exists.
    expect(
      resultSummary({
        session_id: 's_child',
        turn_id: 't1',
        status: 'completed',
        step: 3,
        turn_count: 1,
        children: [],
        expects_wake: false,
        result: 'pong',
      }),
    ).toBe('completed')
    expect(resultSummary({ status: 'awaiting_approval' })).toBe(
      'awaiting approval',
    )
  })

  it('unwraps the iii result envelope', () => {
    expect(
      resultSummary({
        content: [{ type: 'text', text: '...' }],
        details: { issues: [{}, {}] },
      }),
    ).toBe('2 issues')
  })

  it('reports a process exit code', () => {
    expect(resultSummary({ exit_code: 0, stdout: 'ok' })).toBe('exit 0')
  })

  it('stays quiet for shapes it cannot count', () => {
    expect(resultSummary(undefined)).toBeNull()
    expect(resultSummary('done')).toBeNull()
    expect(resultSummary({ a: [1], b: [2] })).toBeNull()
    expect(resultSummary({ error: { kind: 'boom' } })).toBeNull()
  })
})

describe('errorSubtitle', () => {
  it('prefers the message, then kind, then code', () => {
    expect(errorSubtitle({ error: { message: 'timed out after 5s' } })).toBe(
      'timed out after 5s',
    )
    expect(errorSubtitle({ error: { kind: 'not_found' } })).toBe('not_found')
    expect(errorSubtitle({ error: {} })).toBe('Failed')
    expect(errorSubtitle({ error: 'plain' })).toBe('plain')
  })

  it('names a gate denial', () => {
    expect(
      errorSubtitle({
        error: {
          kind: 'denied',
          details: { status: 'denied', denied_by: 'user', reason: 'no' },
        },
      }),
    ).toBe('Denied by user · no')
  })

  it('returns null without an error envelope', () => {
    expect(errorSubtitle({ ok: true })).toBeNull()
    expect(errorSubtitle('text')).toBeNull()
  })
})

describe('formatRowDuration', () => {
  it('uses the shared console scale', () => {
    expect(formatRowDuration(87)).toBe('87ms')
    expect(formatRowDuration(1234)).toBe('1.2s')
    expect(formatRowDuration(0.4)).toBe('400μs')
  })
})
