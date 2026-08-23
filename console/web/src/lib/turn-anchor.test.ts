import { describe, expect, it } from 'vitest'
import type { Message } from '@/types/chat'
import { turnAnchorMessageId } from './turn-anchor'

function user(id: string): Message {
  return { id, role: 'user', content: 'hi', createdAt: 1 }
}

function assistant(id: string): Message {
  return { id, role: 'assistant', content: 'ok', createdAt: 2 }
}

function notice(id: string): Message {
  return { id, role: 'system', content: 'note', createdAt: 2 }
}

describe('turnAnchorMessageId', () => {
  it('lands on the user row that started the turn', () => {
    const messages = [
      user('e_idem_msg-1'),
      assistant('e_t_aaa_0_assistant'),
      user('e_idem_msg-2'),
      assistant('e_t_bbb_0_assistant'),
    ]
    expect(turnAnchorMessageId(messages, 't_bbb')).toBe('e_idem_msg-2')
    expect(turnAnchorMessageId(messages, 't_aaa')).toBe('e_idem_msg-1')
  })

  it('skips local-only notices between the user row and the turn', () => {
    const messages = [
      user('e_idem_msg-1'),
      notice('local-uid-notice'),
      assistant('e_t_aaa_0_assistant'),
    ]
    expect(turnAnchorMessageId(messages, 't_aaa')).toBe('e_idem_msg-1')
  })

  it('stops the backward walk at another durable entry', () => {
    const messages = [
      user('e_idem_msg-1'),
      notice('e_t_aaa_stopped'),
      assistant('e_t_bbb_0_assistant'),
    ]
    // msg-1 belongs to turn aaa's exchange — land on turn bbb itself.
    expect(turnAnchorMessageId(messages, 't_bbb')).toBe('e_t_bbb_0_assistant')
  })

  it('falls back to the turn entry when no user row precedes it', () => {
    const messages = [assistant('e_t_aaa_0_assistant')]
    expect(turnAnchorMessageId(messages, 't_aaa')).toBe('e_t_aaa_0_assistant')
  })

  it('returns null for an unknown turn or empty turn id', () => {
    const messages = [user('e_idem_msg-1'), assistant('e_t_aaa_0_assistant')]
    expect(turnAnchorMessageId(messages, 't_zzz')).toBeNull()
    expect(turnAnchorMessageId(messages, '')).toBeNull()
  })

  it('matches the turn id exactly, never a longer id sharing the prefix', () => {
    const messages = [assistant('e_t_aaab_0_assistant')]
    expect(turnAnchorMessageId(messages, 't_aaa')).toBeNull()
  })
})
