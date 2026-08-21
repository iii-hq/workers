import { describe, expect, it } from 'vitest'
import {
  enqueuePanelCommand,
  type PendingPanelCommand,
} from './panel-command-queue'

describe('workspace panel command queue', () => {
  it('coalesces only duplicate opens from the same originating tab', () => {
    const queue: PendingPanelCommand[] = []

    expect(
      enqueuePanelCommand(queue, {
        type: 'open',
        tabId: 'tab-a',
        screen: 'ext:workers',
      }),
    ).toBe(true)
    expect(
      enqueuePanelCommand(queue, {
        type: 'open',
        tabId: 'tab-a',
        screen: 'ext:workers',
      }),
    ).toBe(false)
    expect(
      enqueuePanelCommand(queue, {
        type: 'open',
        tabId: 'tab-b',
        screen: 'ext:workers',
      }),
    ).toBe(true)

    expect(queue).toHaveLength(2)
  })

  it('keeps every panel addition in FIFO order', () => {
    const queue: PendingPanelCommand[] = []
    const left: PendingPanelCommand = {
      type: 'add',
      tabId: 'tab-a',
      side: 'left',
    }
    const mobile: PendingPanelCommand = {
      type: 'add',
      tabId: 'tab-a',
      side: 'right',
      mobileIndex: 2,
    }

    expect(enqueuePanelCommand(queue, left)).toBe(true)
    expect(enqueuePanelCommand(queue, mobile)).toBe(true)
    expect(queue).toEqual([left, mobile])
  })
})
