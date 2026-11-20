import { describe, expect, it } from 'vitest'
import { swappedPlacement, type WorkspaceTab } from '@/lib/workspace-tabs'
import { workspaceLayoutTransform } from './use-workspace-tabs'

const tab: WorkspaceTab = {
  id: 't',
  columns: 3,
  screens: ['ext:kanban-board', 'chat:d-1', 'chat'],
  paneIds: ['p-kanban', 'p-d1', 'p-chat'],
}

describe('chat draft seeds through layout writes', () => {
  const layout = {
    tabs: [tab],
    activeTabId: 't',
    chatDrafts: { 'd-1': { text: 'KAN-8' }, gone: { text: 'x' } },
  }

  it('keeps the seeds a transform does not mention, dropping only those no column shows', () => {
    // Most transforms build a fresh state without `chatDrafts` (renaming a tab, resizing, …).
    const next = workspaceLayoutTransform((state) => ({
      tabs: state.tabs.map((t) => ({ ...t, name: 'Work' })),
      activeTabId: state.activeTabId,
    }))(layout)
    expect(next.chatDrafts).toEqual({ 'd-1': { text: 'KAN-8' } })
  })

  it("drops a seed when its draft's column goes", () => {
    const next = workspaceLayoutTransform((state) => ({
      ...state,
      tabs: [
        {
          ...tab,
          columns: 2,
          screens: ['ext:kanban-board', 'chat'],
          paneIds: ['p-kanban', 'p-chat'],
        },
      ],
    }))(layout)
    expect(next.chatDrafts).toEqual({})
  })
})

describe('swappedPlacement', () => {
  it('names the pane a reuseBeside swap replaces', () => {
    const swap = swappedPlacement(
      tab,
      'chat:s-2',
      { relativeTo: 'ext:kanban-board', direction: 'right', reuseBeside: true },
      () => 'p-new',
    )
    expect(swap?.replacedPaneId).toBe('p-d1')
    expect(swap?.tab.screens).toEqual(['ext:kanban-board', 'chat:s-2', 'chat'])
  })

  it('names the pane a replaces swap replaces, and nothing when it opens a column instead', () => {
    expect(
      swappedPlacement(tab, 'chat:s-2', { replaces: 'chat:d-1' }, () => 'p-new')
        ?.replacedPaneId,
    ).toBe('p-d1')
    expect(
      swappedPlacement(tab, 'chat:s-2', { relativeTo: 'ext:kanban-board' }),
    ).toBeNull()
  })
})
