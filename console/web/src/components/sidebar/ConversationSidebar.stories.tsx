import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import { PageSidebar } from '@/components/ui/PageChrome'
import type { Conversation } from '@/types/chat'
import { ConversationSidebar } from './ConversationSidebar'

const NOW = Date.now()
const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

function conversation(
  partial: Partial<Conversation> & { id: string; title: string },
): Conversation {
  return {
    model: null,
    messages: [],
    createdAt: NOW - DAY,
    updatedAt: NOW - DAY,
    ...partial,
  }
}

/* Mirrors the shapes the tree renders: profile-tinted roots, a root with
   agent- and trigger-spawned children, live/error status, a plain chat, and
   a title long enough to truncate against the caret. */
const CONVERSATIONS: Conversation[] = [
  conversation({
    id: 'launch',
    title: 'Harness launch',
    updatedAt: NOW - 2 * MIN,
    status: 'working',
    agentProfile: {
      id: 'p-launch',
      name: 'Launch',
      icon: 'code',
      color: 'green',
    },
  }),
  conversation({
    id: 'post-launch',
    title: 'Harness post launch',
    updatedAt: NOW - 25 * MIN,
    agentProfile: {
      id: 'p-post',
      name: 'Post launch',
      icon: 'review',
      color: 'amber',
    },
  }),
  conversation({
    id: 'eval',
    title: 'Harness eval',
    updatedAt: NOW - 3 * HOUR,
    status: 'error',
    statusReason: 'model unavailable',
    agentProfile: { id: 'p-eval', name: 'Eval', icon: 'test', color: 'rose' },
  }),
  conversation({
    id: 'console',
    title: 'Console UX revamp',
    updatedAt: NOW - 5 * HOUR,
    agentProfile: {
      id: 'p-design',
      name: 'Design',
      icon: 'design',
      color: 'blue',
    },
  }),
  conversation({
    id: 'documents',
    title: 'Documents',
    updatedAt: NOW - 8 * HOUR,
    agentProfile: { id: 'p-docs', name: 'Docs', icon: 'docs', color: 'purple' },
  }),
  conversation({
    id: 'documents-search',
    title: 'Testing harness release notes against the changelog',
    parentId: 'documents',
    createdAt: NOW - 7 * HOUR,
    updatedAt: NOW - 7 * HOUR,
    spawnedBy: 'agent',
    subagentAppearance: { name: 'Researcher', icon: 'search', color: 'teal' },
  }),
  conversation({
    id: 'documents-sync',
    title: 'Daily sync',
    parentId: 'documents',
    createdAt: NOW - 6 * HOUR,
    updatedAt: NOW - 6 * HOUR,
    spawnedBy: 'trigger',
  }),
  conversation({
    id: 'documents-roadmap',
    title: 'Roadmap plan',
    parentId: 'documents',
    createdAt: NOW - 5 * HOUR,
    updatedAt: NOW - 5 * HOUR,
    spawnedBy: 'agent',
    subagentAppearance: { name: 'Planner', icon: 'docs', color: 'amber' },
  }),
  conversation({
    id: 'registry',
    title: 'Workers registry',
    updatedAt: NOW - 2 * DAY,
  }),
  conversation({
    id: 'debts',
    title: 'Tech debts',
    updatedAt: NOW - 4 * DAY,
  }),
]

function Sidebar({ narrow, width }: { narrow?: boolean; width: number }) {
  const [activeId, setActiveId] = useState<string | null>('console')
  return (
    <div className="flex h-[560px] overflow-hidden rounded-sm border border-edge">
      <PageSidebar style={{ width }} narrow={narrow}>
        <ConversationSidebar
          conversations={CONVERSATIONS}
          activeId={activeId}
          narrow={narrow}
          onSelect={setActiveId}
          onRename={fn()}
          onRemove={fn()}
        />
      </PageSidebar>
    </div>
  )
}

const meta = {
  title: 'Chat/ConversationSidebar',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

/** The wide pane: 28px rows, tinted glyphs, caret after the label, hover X. */
export const Wide: Story = {
  render: () => <Sidebar width={272} />,
}

/** The list as the whole phone-sized page: touch rows and visible actions. */
export const Narrow: Story = {
  render: () => <Sidebar width={375} narrow />,
}
