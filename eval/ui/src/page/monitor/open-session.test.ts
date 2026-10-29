import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { canOpenSession, openSession } from './open-session'

function fakeHost(parts: {
  openScreen?: (request: { screen: string }) => void
  selectConversation?: (sessionId: string) => void
}): Host {
  return {
    panels: parts.openScreen ? { open: vi.fn(), openScreen: parts.openScreen } : { open: vi.fn() },
    chat: parts.selectConversation ? { selectConversation: parts.selectConversation } : {},
  } as unknown as Host
}

describe('openSession', () => {
  it('opens the conversation beside the page when the console can place screens', () => {
    const openScreen = vi.fn()
    const selectConversation = vi.fn()
    expect(openSession(fakeHost({ openScreen, selectConversation }), 's_root')).toBe(true)
    expect(openScreen).toHaveBeenCalledWith({ screen: 'chat:s_root' })
    expect(selectConversation).not.toHaveBeenCalled()
  })

  it('selects the conversation when the screen is unknown to the console', () => {
    const openScreen = vi.fn(() => {
      throw new Error("panels.openScreen: unknown screen 'chat:s_root'")
    })
    const selectConversation = vi.fn()
    expect(openSession(fakeHost({ openScreen, selectConversation }), 's_root')).toBe(true)
    expect(selectConversation).toHaveBeenCalledWith('s_root')
  })

  it('selects the conversation on a console without openScreen', () => {
    const selectConversation = vi.fn()
    expect(openSession(fakeHost({ selectConversation }), 'eval_monitor_ev_1')).toBe(true)
    expect(selectConversation).toHaveBeenCalledWith('eval_monitor_ev_1')
  })

  it('reports nothing opened when the screen throws and there is no fallback', () => {
    const openScreen = vi.fn(() => {
      throw new Error('unknown screen')
    })
    expect(openSession(fakeHost({ openScreen }), 's_root')).toBe(false)
    expect(openSession(fakeHost({}), 's_root')).toBe(false)
  })
})

describe('canOpenSession', () => {
  it('needs either way of showing a conversation', () => {
    expect(canOpenSession(fakeHost({ openScreen: vi.fn() }))).toBe(true)
    expect(canOpenSession(fakeHost({ selectConversation: vi.fn() }))).toBe(true)
    expect(canOpenSession(fakeHost({}))).toBe(false)
  })
})
