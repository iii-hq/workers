import { describe, expect, it } from 'vitest'
import type { Conversation } from '@/types/chat'
import { filterConversations } from './conversation-filter'

let clock = 0
function conv(id: string, title: string, at = clock++): Conversation {
  return {
    id,
    title,
    model: null,
    mode: 'agent',
    messages: [],
    createdAt: at,
    updatedAt: at,
  }
}

const list = [
  conv('a', 'fix harness todo'),
  conv('b', 'Console Search Bar'),
  conv('c', 'shell deny policy'),
]

describe('filterConversations', () => {
  it('matches case-insensitively on title substring', () => {
    expect(filterConversations(list, 'search').map((c) => c.id)).toEqual(['b'])
    expect(filterConversations(list, 'CONSOLE').map((c) => c.id)).toEqual(['b'])
  })

  it('trims the query and returns the input unchanged when empty', () => {
    expect(filterConversations(list, '')).toBe(list)
    expect(filterConversations(list, '   ')).toBe(list)
    expect(filterConversations(list, '  shell ').map((c) => c.id)).toEqual([
      'c',
    ])
  })

  it('preserves input order and returns empty on no match', () => {
    expect(filterConversations(list, 'o').map((c) => c.id)).toEqual([
      'a',
      'b',
      'c',
    ])
    expect(filterConversations(list, 'zzz')).toEqual([])
  })
})
