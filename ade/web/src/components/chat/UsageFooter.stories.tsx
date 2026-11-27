import type { Meta, StoryObj } from '@storybook/react-vite'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import {
  rawUsage,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import { UsageFooter } from './UsageFooter'

const NOW = Date.now()

/** Work at 85 % (amber track) and Personal at 100 % (closed arc, red track). */
function levels() {
  const week = (used_percent: number) => ({
    used_percent,
    resets_at_ms: NOW + 3_600_000,
  })
  return {
    accounts: rawUsage(NOW).claude.accounts.map((a) =>
      a.id === 'default'
        ? { ...a, usage: { ...a.usage, week: week(85) } }
        : a.id === 'personal'
          ? { ...a, usage: { ...a.usage, week: week(100) } }
          : a,
    ),
  }
}

function seed(suggestion: boolean, withLevels: boolean) {
  setChatCapabilitiesForTests({
    harnessRetry: true,
    tmuxAccounts: true,
    tmuxCredential: true,
  })
  seedClaudeAccountsForTests({
    usage: usageFixture(
      NOW,
      withLevels
        ? levels()
        : suggestion
          ? {
              suggestion: {
                id: 'sug_1',
                from: 'default',
                to: 'side-project',
                reason: 'week_90',
                following: 2,
                at_ms: NOW,
              },
            }
          : {},
    ),
  })
}

/* The sidebar's usage footer (§4.3.3, D178): the Claude Code mark and a
   ring per account's week, global first, in a sidebar-wide column; the Acme
   demo's accounts, never real ones. Hover a ring for its line. */
const meta = {
  title: 'Chat/UsageFooter',
  component: UsageFooter,
  parameters: { layout: 'padded' },
  decorators: [
    (Story, context) => {
      seed(
        context.parameters.suggestion === true,
        context.parameters.levels === true,
      )
      return (
        <div className="flex h-96 w-64 flex-col justify-end rounded-lg border border-border bg-sidebar">
          <Story />
        </div>
      )
    },
  ],
} satisfies Meta<typeof UsageFooter>

export default meta
type Story = StoryObj<typeof meta>

/** A ring per account; Personal's 5-hour limit doesn't show here, Side project (not signed in) is a bare track. */
export const Default: Story = {}

/** Work at 85 % (amber track), Personal at 100 % (closed arc, red track). */
export const HighAndFull: Story = { parameters: { levels: true } }

/** tmux suggests a switch: "Sign in to Side project" (it isn't signed in). */
export const WithSuggestion: Story = { parameters: { suggestion: true } }
