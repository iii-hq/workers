import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import { ProjectMenu } from '@/components/projects/ProjectMenu'
import { PageSidebar } from '@/components/ui/PageChrome'
import type { ChatProject } from '@/lib/backend/projects'
import type { Conversation } from '@/types/chat'
import { ConversationGroupHeader } from './ConversationGroupHeader'
import { ConversationRow } from './ConversationRow'
import { ConversationSidebar } from './ConversationSidebar'
import './conversation-tree.css'

/* The Acme demo of the chat-projects boards (Main, Phone-Sessions): never
   data from a real environment. */
const NOW = Date.now()
const MIN = 60_000
const HOUR = 60 * MIN

const ACME: ChatProject = {
  id: 'prj_acme_web',
  name: 'acme-web',
  color: 'blue',
  folders: [
    '/home/me/work/acme/web',
    '/home/me/work/acme/api',
    '/home/me/work/acme/infra',
  ],
  created_at_ms: NOW - 30 * HOUR,
  updated_at_ms: NOW - 30 * HOUR,
}
const BILLING: ChatProject = {
  id: 'prj_billing',
  name: 'billing-api',
  color: 'purple',
  folders: ['/home/me/work/billing'],
  created_at_ms: NOW - 40 * HOUR,
  updated_at_ms: NOW - 40 * HOUR,
}
const MOBILE: ChatProject = {
  id: 'prj_mobile',
  name: 'mobile',
  color: 'teal',
  folders: ['/home/me/work/mobile'],
  created_at_ms: NOW - 50 * HOUR,
  updated_at_ms: NOW - 50 * HOUR,
}

function chat(
  partial: Partial<Conversation> & { id: string; title: string },
): Conversation {
  return {
    model: null,
    messages: [],
    createdAt: NOW - HOUR,
    updatedAt: NOW - HOUR,
    ...partial,
  }
}

const WEBHOOKS = chat({
  id: 'webhooks',
  title: 'Webhook retries',
  projectId: ACME.id,
  workingDir: '/home/me/work/acme/web',
  description: 'Backoff with jitter for failed deliveries, then a PR',
  descriptionBy: 'agent',
  descriptionAtMs: NOW - 2 * MIN,
  updatedAt: NOW - 2 * MIN,
})

const CONVERSATIONS: Conversation[] = [
  WEBHOOKS,
  chat({
    id: 'flaky',
    title: 'Flaky e2e on CI',
    projectId: ACME.id,
    workingDir: '/home/me/work/acme/web',
    status: 'done',
    statusReason: 'awaiting_answer',
    updatedAt: NOW - HOUR,
  }),
  chat({
    id: 'checkout',
    title: 'Checkout copy pass',
    projectId: ACME.id,
    workingDir: '/home/me/work/acme/web',
    description: 'Shorter error messages on the payment form',
    descriptionBy: 'user',
    updatedAt: NOW - 3 * HOUR,
  }),
  chat({
    id: 'infra',
    title: 'Terraform drift',
    projectId: ACME.id,
    workingDir: '/home/me/work/acme/infra',
    updatedAt: NOW - 5 * HOUR,
  }),
  chat({
    id: 'invoices',
    title: 'Invoice rounding',
    projectId: BILLING.id,
    workingDir: '/home/me/work/billing',
    updatedAt: NOW - 6 * HOUR,
  }),
  chat({
    id: 'regex',
    title: 'Regex for log lines',
    workingDir: '/home/me/scratch',
    updatedAt: NOW - 30 * HOUR,
  }),
]

function Sidebar({ narrow, width }: { narrow?: boolean; width: number }) {
  const [activeId, setActiveId] = useState<string | null>('webhooks')
  return (
    <div className="flex h-[620px] overflow-hidden rounded-sm border border-edge">
      <PageSidebar style={{ width }} narrow={narrow}>
        <ConversationSidebar
          conversations={CONVERSATIONS}
          activeId={activeId}
          narrow={narrow}
          chatProjects={[ACME, BILLING, MOBILE]}
          onCreate={fn()}
          onNewProject={fn()}
          onAddProject={fn()}
          onCreateInProject={fn()}
          onEditProject={fn()}
          onSaveDetails={async () => {}}
          onMoveToProject={async () => {}}
          agentSwitch
          onSelect={setActiveId}
          onRename={fn()}
          onRemove={fn()}
        />
      </PageSidebar>
    </div>
  )
}

const meta = {
  title: 'Chat/Projects',
  parameters: { layout: 'padded' },
} satisfies Meta

export default meta
type Story = StoryObj

/** The list grouped by project: 3 newest, "1 more", an empty project, No project last. */
export const ListByProject: Story = {
  render: () => <Sidebar width={300} />,
}

/** The list as the whole phone page: the header counts conversations. */
export const ListByProjectNarrow: Story = {
  render: () => <Sidebar width={375} narrow />,
}

/** One project header: dot, name, "3 folders", the "+" and the "⋯" menu. */
export const ProjectHeader: Story = {
  render: () => (
    <div className="w-[300px] rounded-sm border border-edge bg-sidebar p-2">
      <div className="iii-ui-tree conversation-tree">
        <ConversationGroupHeader
          label={ACME.name}
          count={4}
          collapsed={false}
          onToggle={fn()}
          chatProject={ACME}
          onCreateIn={fn()}
          menu={
            <ProjectMenu
              project={ACME}
              conversationCount={4}
              onNewConversation={fn()}
              onEdit={fn()}
              remove={async () => {}}
            />
          }
        />
      </div>
    </div>
  ),
}

/** A row with the agent's description on line 2, and one waiting for you. */
export const RowWithDescription: Story = {
  render: () => (
    <div className="w-[300px] rounded-sm border border-edge bg-sidebar p-2">
      <div className="iii-ui-tree conversation-tree">
        <ConversationRow
          conversation={WEBHOOKS}
          active
          onSelect={fn()}
          onRename={fn()}
          onRemove={fn()}
          secondLine="full"
          projects={[ACME, BILLING]}
          onMoveToProject={fn()}
        />
        <ConversationRow
          conversation={CONVERSATIONS[1]}
          active={false}
          onSelect={fn()}
          onRename={fn()}
          onRemove={fn()}
          secondLine="full"
        />
        <ConversationRow
          conversation={CONVERSATIONS[5]}
          active={false}
          onSelect={fn()}
          onRename={fn()}
          onRemove={fn()}
          secondLine="full"
          folderTail
        />
      </div>
    </div>
  ),
}
