// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  loadPinnedConversationIds,
  movePinnedId,
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

  it('moves a pin before or after another, ignoring no-op drops', () => {
    const ids = ['a', 'b', 'c', 'd']
    expect(movePinnedId(ids, 'd', 'a', 'before')).toEqual(['d', 'a', 'b', 'c'])
    expect(movePinnedId(ids, 'a', 'c', 'after')).toEqual(['b', 'c', 'a', 'd'])
    expect(movePinnedId(ids, 'b', 'c', 'before')).toBe(ids)
    expect(movePinnedId(ids, 'b', 'b', 'after')).toBe(ids)
    expect(movePinnedId(ids, 'x', 'a', 'before')).toBe(ids)
  })
})
