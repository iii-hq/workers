import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { limitCardModel } from '@/lib/limit-card'
import {
  accountsData,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import type { SystemMessage } from '@/types/chat'
import { LimitCard } from './LimitCard'

const NOW = Date.now()
const data = accountsData(usageFixture(NOW))

function failure(extra: SystemMessage['failure']): SystemMessage {
  return {
    id: 'e-limit',
    role: 'system',
    kind: 'turn-failure',
    content: 'This Claude Code account reached its usage limit',
    createdAt: NOW,
    failure: extra,
  }
}

function model(failed: SystemMessage | null) {
  const m = limitCardModel({
    sessionId: 'story',
    failure: failed,
    idle: true,
    resolved: 'personal',
    account: data.account,
  })
  if (!m) throw new Error('no limit card')
  return m
}

const FAILED = model(
  failure({
    usageLimit: {
      window: 'five_hour',
      resetsAtMs: NOW + 3_600_000,
      account: 'personal',
    },
  }),
)

/* The limit card above the composer (§4.3.4), the Acme demo's accounts:
   Personal at its 5-hour limit, Work with room. */
const meta = {
  title: 'Chat/LimitCard',
  component: LimitCard,
  parameters: { layout: 'padded' },
  decorators: [
    (Story) => (
      <div className="mx-auto max-w-2xl">
        <Story />
      </div>
    ),
  ],
  args: {
    model: FAILED,
    data,
    time: { now: NOW },
    onContinue: fn(async () => {}),
    onMakeGlobal: fn(async () => {}),
    onOpenAccounts: fn(),
    onDismiss: fn(),
  },
} satisfies Meta<typeof LimitCard>

export default meta
type Story = StoryObj<typeof meta>

/** The last turn hit Personal's 5-hour limit: continue in Work, or another. */
export const TurnFailed: Story = {}

/** The conversation follows the global: "Make Work the global" too. */
export const FollowsGlobal: Story = { args: { followsGlobal: true } }

/** Idle on an account whose reading is at 100 %. */
export const ReadingFull: Story = { args: { model: model(null) } }

/** A side chat keeps its account (D2): the title, the reset and the X. */
export const SideChat: Story = { args: { sideChat: true } }

/** A pin to an account removed from tmux. */
export const UnknownAccount: Story = {
  args: { model: model(failure({ unknownAccount: 'old-team' })) },
}
