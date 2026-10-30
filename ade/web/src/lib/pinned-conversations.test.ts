// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  loadPinnedConversationIds,
  PINNED_CONVERSATIONS_KEY,
  savePinnedConversationIds,
  togglePinnedId,
} from './pinned-conversations'

beforeEach(() => {
  window.localStorage.clear()
})

describe('pinned conversations', () => {
  it('round-trips ids and clears the key when empty', () => {
    savePinnedConversationIds(['b', 'a', 'b'])
    expect(loadPinnedConversationIds()).toEqual(['b', 'a'])
    savePinnedConversationIds([])
    expect(window.localStorage.getItem(PINNED_CONVERSATIONS_KEY)).toBeNull()
  })

  it('ignores a corrupt store', () => {
    window.localStorage.setItem(PINNED_CONVERSATIONS_KEY, '{nope')
    expect(loadPinnedConversationIds()).toEqual([])
    window.localStorage.setItem(PINNED_CONVERSATIONS_KEY, '[1, "x"]')
    expect(loadPinnedConversationIds()).toEqual(['x'])
  })

  it('toggles with the newest pin first', () => {
    expect(togglePinnedId([], 'a')).toEqual(['a'])
    expect(togglePinnedId(['a'], 'b')).toEqual(['b', 'a'])
    expect(togglePinnedId(['b', 'a'], 'b')).toEqual(['a'])
  })
})
