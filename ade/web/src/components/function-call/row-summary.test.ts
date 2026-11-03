import { describe, expect, it } from 'vitest'
import { errorSubtitle, formatRowDuration, resultSummary } from './row-summary'

describe('resultSummary', () => {
  it('counts a bare list and a single array field', () => {
    expect(resultSummary([1, 2, 3, 4])).toBe('4 results')
    expect(resultSummary({ files: [{}, {}, {}] })).toBe('3 files')
    expect(resultSummary({ matches: [{}] })).toBe('1 match')
    expect(resultSummary({ entries: [{}, {}] })).toBe('2 entries')
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
