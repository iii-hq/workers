import { describe, expect, it } from 'vitest'
import {
  closeChatTab,
  NO_OPEN_CHAT_TABS,
  type OpenChatTabs,
  openChatTab,
  pruneChatTabs,
} from './open-chat-tabs'

const open = (...ids: string[]) =>
  ids.reduce<OpenChatTabs>(
    (state, id) => openChatTab(state, id, 3),
    NO_OPEN_CHAT_TABS,
  )

describe('open chat tabs', () => {
  it('appends a chat you open and keeps the order when you revisit one', () => {
    const state = open('a', 'b', 'a')
    expect(state.ids).toEqual(['a', 'b'])
    expect(state.recency).toEqual(['b', 'a'])
  })

  it('past the cap, closes the least recently used tab, never the open one', () => {
    // a, b, c open; a used last; d opens → b (stalest) leaves.
    const state = open('a', 'b', 'c', 'a', 'd')
    expect(state.ids).toEqual(['a', 'c', 'd'])
  })

  it('closing the open tab hands over to the right, else the left', () => {
    const state = open('a', 'b', 'c')
    expect(closeChatTab(state, 'b')).toMatchObject({
      next: 'c',
      state: { ids: ['a', 'c'] },
    })
    expect(closeChatTab(state, 'c').next).toBe('b')
    expect(closeChatTab(open('a'), 'a').next).toBeNull()
  })

  it('drops tabs whose chat is gone', () => {
    const state = pruneChatTabs(open('a', 'b', 'c'), (id) => id !== 'b')
    expect(state).toEqual({ ids: ['a', 'c'], recency: ['a', 'c'] })
  })
})
