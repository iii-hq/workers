import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import {
  type GlobalResult,
  parseUsageSnapshot,
} from '@/lib/backend/claude-accounts'
import { limitCardModel } from '@/lib/limit-card'
import {
  accountsData,
  rawUsage,
  usageFixture,
} from '@/stories/fixtures/claude-accounts-fixtures'
import type { SystemMessage } from '@/types/chat'
import { LimitCard } from './LimitCard'

const NOW = Date.now()
const HOUR = 3_600_000
const data = accountsData(usageFixture(NOW))
/** Personal the global one, tmux waiting for 2 windows pinned to Work. */
const waiting = accountsData(
  usageFixture(NOW, {
    global: 'personal',
    pending_global: {
      account: 'default',
      by: 'person',
      waiting_for: ['%3', '%5'],
      at_ms: NOW,
    },
  }),
)
/** Work's week full too: no account has room. */
function noRoomData() {
  const raw = rawUsage(NOW)
  const work = raw.claude.accounts[0] as { usage: { week: unknown } }
  work.usage.week = { used_percent: 100, resets_at_ms: NOW + 50 * HOUR }
  return accountsData(parseUsageSnapshot(raw))
}

const switched: GlobalResult = {
  global: 'default',
  state: 'switched',
  following: 1,
  restarted: [],
  afterTurn: [],
  waitingFor: [],
}

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

function model(failed: SystemMessage | null, now = NOW) {
  const m = limitCardModel({
    sessionId: 'story',
    failure: failed,
    idle: true,
    resolved: 'personal',
    account: data.account,
    now,
  })
  if (!m) throw new Error('no limit card')
  return m
}

const FAILED_TURN = failure({
  usageLimit: {
    window: 'five_hour',
    resetsAtMs: NOW + HOUR,
    account: 'personal',
  },
})
const FAILED = model(FAILED_TURN)

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
    onPin: fn(async () => {}),
    onSetGlobal: fn(async () => switched),
    onRetry: fn(async () => {}),
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

/** Make Work the global while tmux waits for windows pinned to it. */
export const WaitingForGlobal: Story = {
  args: { data: waiting, followsGlobal: true },
}

/** No account has room: no "Continue in"; Other… lists them with their use. */
export const NoAccountWithRoom: Story = { args: { data: noRoomData() } }

/** The window reset since: "The limit has reset" and "Try again". */
export const LimitReset: Story = {
  args: { model: model(FAILED_TURN, NOW + 2 * HOUR) },
}

/** Idle on an account whose reading is at 100 %. */
export const ReadingFull: Story = { args: { model: model(null) } }

/** A side chat keeps its account (D2): the title, the reset and the X. */
export const SideChat: Story = { args: { sideChat: true } }

/** A pin to an account removed from tmux. */
export const UnknownAccount: Story = {
  args: { model: model(failure({ unknownAccount: 'old-team' })) },
}
