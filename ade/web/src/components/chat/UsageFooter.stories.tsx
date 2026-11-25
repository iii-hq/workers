import type { Meta, StoryObj } from '@storybook/react-vite'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import { usageFixture } from '@/stories/fixtures/claude-accounts-fixtures'
import { UsageFooter } from './UsageFooter'

const NOW = Date.now()

function seed(suggestion: boolean) {
  setChatCapabilitiesForTests({
    harnessRetry: true,
    tmuxAccounts: true,
    tmuxCredential: true,
  })
  seedClaudeAccountsForTests({
    usage: usageFixture(
      NOW,
      suggestion
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

/* The sidebar's "Usage this week" (§4.3.3), in a sidebar-wide column: the
   Acme demo's accounts, never real ones. */
const meta = {
  title: 'Chat/UsageFooter',
  component: UsageFooter,
  parameters: { layout: 'padded' },
  decorators: [
    (Story, context) => {
      seed(context.parameters.suggestion === true)
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

/** A mini meter per account; Personal's 5-hour limit doesn't show here. */
export const Default: Story = {}

/** tmux suggests a switch: "Sign in to Side project" (it isn't signed in). */
export const WithSuggestion: Story = { parameters: { suggestion: true } }
