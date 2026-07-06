import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type { WorktreeInfo } from '@/lib/worktrees'
import { worktreeGraphFixtures } from '@/stories/fixtures/worktree-fixtures'
import { WorktreeDetailPanel } from './WorktreeDetailPanel'
import { WorktreeGraph } from './WorktreeGraph'

function GraphHarness({ worktrees }: { worktrees: WorktreeInfo[] }) {
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const selected = worktrees.find((w) => w.worktree_id === selectedId) ?? null
  return (
    <div className="flex min-h-[420px] border border-rule bg-bg">
      <div className="flex-1 overflow-auto p-4">
        <WorktreeGraph
          worktrees={worktrees}
          selectedId={selectedId}
          onSelect={(id) => setSelectedId((cur) => (cur === id ? null : id))}
        />
      </div>
      {selected ? (
        <WorktreeDetailPanel
          worktree={selected}
          onClose={() => setSelectedId(null)}
        />
      ) : null}
    </div>
  )
}

const meta = {
  title: 'pages/WorktreeGraph',
  component: GraphHarness,
} satisfies Meta<typeof GraphHarness>

export default meta
type Story = StoryObj<typeof meta>

export const TwoReposEveryLifecycle: Story = {
  name: 'two repos, every lifecycle',
  args: { worktrees: worktreeGraphFixtures },
}

export const SingleWorktree: Story = {
  name: 'single worktree',
  args: { worktrees: worktreeGraphFixtures.slice(0, 1) },
}
