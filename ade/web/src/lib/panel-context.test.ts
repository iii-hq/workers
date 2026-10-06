import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getPanelContext,
  requestPanelOpen,
  requestScreenOpen,
  resetPanelContextForTests,
  subscribePanelOpen,
  subscribeScreenOpen,
} from './panel-context'

beforeEach(resetPanelContextForTests)

describe('panel context bridge', () => {
  it('stores context before notifying the workspace', () => {
    const listener = vi.fn((event) => {
      expect(getPanelContext('ide')).toBe(event)
    })
    const off = subscribePanelOpen(listener)
    const event = requestPanelOpen({
      pageId: 'ide',
      context: { type: 'file', path: '/repo/a.ts' },
    })

    expect(listener).toHaveBeenCalledWith(event)
    expect(event.id).toBe(1)
    off()
  })

  it('emits a fresh event for repeated context', () => {
    const first = requestPanelOpen({ pageId: 'ide', context: null })
    const second = requestPanelOpen({ pageId: 'ide', context: null })
    expect(second.id).toBe(first.id + 1)
    expect(getPanelContext('ide')).toBe(second)
  })

  it('hands a screen request to every workspace listener until it unsubscribes', () => {
    const listener = vi.fn()
    const off = subscribeScreenOpen(listener)
    const request = { screen: 'traces', relativeTo: 'ext:onboarding' }
    requestScreenOpen(request)
    expect(listener).toHaveBeenCalledWith(request)
    off()
    requestScreenOpen(request)
    expect(listener).toHaveBeenCalledTimes(1)
  })
})
