import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getPanelContext,
  requestPanelOpen,
  resetPanelContextForTests,
  subscribePanelOpen,
} from './panel-context'

beforeEach(resetPanelContextForTests)

describe('panel context bridge', () => {
  it('stores context before notifying the workspace', () => {
    const listener = vi.fn((event) => {
      expect(getPanelContext('shell')).toBe(event)
    })
    const off = subscribePanelOpen(listener)
    const event = requestPanelOpen({
      pageId: 'shell',
      context: { type: 'file', path: '/repo/a.ts' },
    })

    expect(listener).toHaveBeenCalledWith(event)
    expect(event.id).toBe(1)
    off()
  })

  it('emits a fresh event for repeated context', () => {
    const first = requestPanelOpen({ pageId: 'shell', context: null })
    const second = requestPanelOpen({ pageId: 'shell', context: null })
    expect(second.id).toBe(first.id + 1)
    expect(getPanelContext('shell')).toBe(second)
  })
})
