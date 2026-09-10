import { describe, expect, it } from 'vitest'
import { traceChatLink } from './traceChatLink'

describe('traceChatLink', () => {
  it('resolves from merged trace tags alone', () => {
    const link = traceChatLink(
      {
        'iii.session.id': 'console-1',
        'iii.session.name': 'my chat',
        'iii.message.id': 't_abc',
      },
      [],
    )
    expect(link).toEqual({
      sessionId: 'console-1',
      sessionName: 'my chat',
      turnId: 't_abc',
    })
  })

  it('falls back to span attributes when tags are absent (deep-linked detail)', () => {
    const link = traceChatLink(undefined, [
      { attributes: { 'other.key': 'x' } },
      { attributes: { 'iii.session.id': 'console-2' } },
      { attributes: { 'iii.message.id': 't_def' } },
    ])
    expect(link).toEqual({
      sessionId: 'console-2',
      sessionName: undefined,
      turnId: 't_def',
    })
  })

  it('trace tags win over span attributes', () => {
    const link = traceChatLink({ 'iii.session.id': 'console-tags' }, [
      { attributes: { 'iii.session.id': 'console-span' } },
    ])
    expect(link?.sessionId).toBe('console-tags')
  })

  it('returns null when no span carries a session id', () => {
    expect(traceChatLink(undefined, [{ attributes: { a: 'b' } }])).toBeNull()
    expect(traceChatLink({}, [])).toBeNull()
  })

  it('ignores empty and non-string values', () => {
    const link = traceChatLink({ 'iii.session.id': '' }, [
      { attributes: { 'iii.session.id': 42 } },
      { attributes: { 'iii.session.id': 'console-3' } },
    ])
    expect(link?.sessionId).toBe('console-3')
  })
})
