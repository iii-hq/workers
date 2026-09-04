import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { bindingsFor } from '@/lib/keybindings/registry'
import { moveItem } from '@/lib/reorder'
import {
  adjacentTabId,
  type WorkspaceTab,
  withTabClosed,
} from '@/lib/workspace-tabs'
import { TabStrip } from './TabStrip'

const EXT_TITLES = new Map([
  ['shell', 'IDE'],
  ['browser', 'Browser'],
  ['github', 'GitHub'],
  ['database', 'Database'],
])

function tabsOf(count: number): WorkspaceTab[] {
  const screens = [
    'traces',
    'workers',
    'ext:shell',
    'ext:browser',
    'ext:github',
    'ext:database',
  ]
  return Array.from({ length: count }, (_, index) => ({
    id: `tab-${index + 1}`,
    columns: 2 as const,
    screens: ['chat', screens[index % screens.length]],
    ...(index === 1 ? { name: 'Release checklist' } : {}),
  }))
}

interface LiveStripProps {
  initial: WorkspaceTab[]
  width?: number
}

/** A strip wired to a local model so every interaction works in the story. */
function LiveStrip({ initial, width = 720 }: LiveStripProps) {
  const [tabs, setTabs] = useState(initial)
  const [activeTabId, setActiveTabId] = useState(initial[0].id)
  return (
    <div className="flex flex-col gap-4">
      <div
        className="flex h-14 items-center gap-3 rounded-sm bg-bg px-3"
        style={{ width }}
      >
        <TabStrip
          tabs={tabs}
          activeTabId={activeTabId}
          extPageTitles={EXT_TITLES}
          onActivate={setActiveTabId}
          onClose={(id) => {
            const next = withTabClosed({ tabs, activeTabId }, id)
            setTabs(next.tabs)
            setActiveTabId(next.activeTabId)
          }}
          onCreate={() => {
            const tab = {
              id: `tab-${Date.now()}`,
              columns: 1 as const,
              screens: [],
            }
            setTabs([...tabs, tab])
            setActiveTabId(tab.id)
          }}
          onRename={(id, name) =>
            setTabs(tabs.map((tab) => (tab.id === id ? { ...tab, name } : tab)))
          }
          onReorder={(from, to) => setTabs(moveItem(tabs, from, to))}
        />
      </div>
      <p className="font-sans text-[12px] text-ink-faint">
        Arrow keys move between workspaces, Delete closes the focused one, a
        middle click closes the one under the pointer, double-click renames.
        Next with{' '}
        <KeyCombo
          binding={bindingsFor('workspace.next', 'mac')[0]}
          platform="mac"
        />
        :{' '}
        <button
          type="button"
          className="underline"
          onClick={() => {
            const id = adjacentTabId(tabs, activeTabId, 1)
            if (id) setActiveTabId(id)
          }}
        >
          go
        </button>
      </p>
    </div>
  )
}

const meta = {
  title: 'Workspace/TabStrip',
  component: LiveStrip,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof LiveStrip>

export default meta
type Story = StoryObj<typeof meta>

export const FewTabs: Story = {
  args: { initial: tabsOf(3) },
}

export const Overflowing: Story = {
  args: { initial: tabsOf(14) },
}

export const NarrowPane: Story = {
  args: { initial: tabsOf(6), width: 360 },
}

export const SingleTab: Story = {
  args: { initial: tabsOf(1) },
}
