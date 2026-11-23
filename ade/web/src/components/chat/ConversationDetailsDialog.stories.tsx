import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import type { ChatProject } from '@/lib/backend/projects'
import type { ConversationDetailsDialog } from './ConversationDetailsDialog'

const NOW = Date.now()
const MIN = 60_000

const PROJECTS: ChatProject[] = [
  {
    id: 'prj_acme_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/home/me/work/acme/web'],
    created_at_ms: NOW,
    updated_at_ms: NOW,
  },
]

/* The Session-Details board without the auto-compact and account rows
   (phases 3 and 4): the Acme demo, never real sessions. */
const meta = {
  title: 'Chat/ConversationDetailsDialog',
  parameters: { layout: 'centered' },
  args: {
    open: true,
    onOpenChange: fn(),
    onSave: async () => {},
    projects: PROJECTS,
    agentSwitch: true,
    conversation: {
      title: 'Webhook retries',
      description: 'Backoff with jitter for failed deliveries, then a PR',
      descriptionBy: 'agent',
      descriptionAtMs: NOW - 2 * MIN,
      projectId: 'prj_acme_web',
      sessionMetadata: {
        description_history: [
          {
            text: 'Mapping every webhook sender',
            by: 'agent',
            at_ms: NOW - 9 * MIN,
          },
          {
            text: 'Reading the delivery code',
            by: 'agent',
            at_ms: NOW - 11 * MIN,
          },
        ],
      },
    },
  },
} satisfies Meta<typeof ConversationDetailsDialog>

export default meta
type Story = StoryObj<typeof meta>

/** The agent wrote the description: the pill, the switch on, recent ones. */
export const ByAgent: Story = {}

/** An engine without `harness::set-description` or projects: name and description only. */
export const OlderEngine: Story = {
  args: {
    agentSwitch: false,
    projects: undefined,
    conversation: {
      title: 'Regex for log lines',
      description: undefined,
      sessionMetadata: {},
    },
  },
}
