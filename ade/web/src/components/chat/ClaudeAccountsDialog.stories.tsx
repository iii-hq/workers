import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { seedChatProjectsForTests } from '@/hooks/use-chat-projects'
import {
  type ClaudeAccountsGate,
  seedClaudeAccountsForTests,
} from '@/lib/backend/claude-accounts'
import type { ChatProject } from '@/lib/backend/projects'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import {
  splitsFixture,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import { ClaudeAccountsDialog } from './ClaudeAccountsDialog'

const NOW = Date.now()

const PROJECTS: ChatProject[] = [
  {
    id: 'prj_acme_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/home/me/work/acme/web', '/home/me/work/acme/api'],
    account: { mode: 'global' },
    created_at_ms: NOW,
    updated_at_ms: NOW,
  },
  {
    id: 'prj_billing',
    name: 'billing-api',
    color: 'green',
    folders: ['/home/me/work/billing'],
    account: { mode: 'pinned', account_id: 'personal' },
    created_at_ms: NOW,
    updated_at_ms: NOW,
  },
]

/** The stores as the engine would fill them, without reading any engine. */
function seed(gate: ClaudeAccountsGate) {
  setChatCapabilitiesForTests({
    harnessRetry: true,
    tmuxAccounts: gate !== 'install',
    tmuxCredential: gate === 'full',
    projects: false,
  })
  seedChatProjectsForTests(PROJECTS)
  seedClaudeAccountsForTests({
    usage: usageFixture(NOW),
    splits: splitsFixture(NOW),
    gate,
  })
}

/* "Claude Code accounts" (§4.3.1): the Acme demo's three accounts, never
   real ones. Actions do nothing here. */
const meta = {
  title: 'Chat/ClaudeAccountsDialog',
  component: ClaudeAccountsDialog,
  parameters: { layout: 'fullscreen' },
  decorators: [
    (Story, context) => {
      seed((context.parameters.gate as ClaudeAccountsGate) ?? 'full')
      return <Story />
    },
  ],
  args: { open: true, onOpenChange: fn() },
} satisfies Meta<typeof ClaudeAccountsDialog>

export default meta
type Story = StoryObj<typeof meta>

/** Accounts and usage: the cards, their meters and the week by project. */
export const Accounts: Story = {}

/** The Projects tab: each project's account and what it does when low. */
export const Projects: Story = { args: { initialTab: 'projects' } }

/** tmux without the credential call: "Update tmux". */
export const NeedsNewerTmux: Story = { parameters: { gate: 'update' } }

/** No tmux on the engine: "Install tmux". */
export const NoTmux: Story = { parameters: { gate: 'install' } }
