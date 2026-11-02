// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest'
import {
  DEFAULT_NEW_CHAT_MODE,
  loadNewChatMode,
  saveNewChatMode,
} from './new-chat-mode'

afterEach(() => localStorage.clear())

describe('new chat mode', () => {
  it('opens a blank chat until the person picks the dialog', () => {
    expect(DEFAULT_NEW_CHAT_MODE).toBe('page')
    expect(loadNewChatMode()).toBe('page')
    saveNewChatMode('dialog')
    expect(loadNewChatMode()).toBe('dialog')
  })

  it('reads an unknown stored value as the default', () => {
    localStorage.setItem('iii-new-chat-mode', 'sheet')
    expect(loadNewChatMode()).toBe('page')
  })

  it('tells this tab when the choice changes', () => {
    let heard = 0
    const onChange = () => {
      heard += 1
    }
    window.addEventListener('iii:new-chat-mode', onChange)
    saveNewChatMode('dialog')
    window.removeEventListener('iii:new-chat-mode', onChange)
    expect(heard).toBe(1)
  })
})
