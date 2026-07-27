import type { Meta, StoryObj } from '@storybook/react-vite'
import type { TurnUsage } from '@/lib/session-usage'
import { TurnUsageChip } from './TurnUsageChip'

const totals = (
  over: Partial<TurnUsage['totals']> = {},
): TurnUsage['totals'] => ({
  input: 412_908,
  output: 18_204,
  cacheRead: 388_110,
  cacheWrite: 12_400,
  reasoning: 0,
  costUsd: 0.0482,
  reported: {
    input: 3,
    output: 3,
    cacheRead: 3,
    cacheWrite: 3,
    reasoning: 0,
    cost: 3,
  },
  total: 431_112,
  ...over,
})

/** Three steps of one tool loop: cold prompt, then two cache-warm calls. */
const warmingTurn: TurnUsage = {
  turnId: 't_9f1a',
  steps: 3,
  stepUsage: [
    {
      entryId: 'e_t_9f1a_0_assistant',
      usage: { input: 12_404, output: 288, cache_read: 0, cost_usd: 0.0021 },
    },
    {
      entryId: 'e_t_9f1a_1_assistant',
      usage: {
        input: 12_910,
        output: 1_044,
        cache_read: 12_400,
        cost_usd: 0.0038,
      },
    },
    {
      entryId: 'e_t_9f1a_2_assistant',
      usage: {
        input: 13_882,
        output: 402,
        cache_read: 12_400,
        cost_usd: 0.0029,
      },
    },
  ],
  totals: totals(),
  functionCalls: 2,
  functionCallErrors: 0,
  startedAt: 0,
  endedAt: 62_000,
  durationMs: 62_000,
  anchorId: 'e_t_9f1a_2_assistant',
  streaming: false,
}

const meta = {
  title: 'chat/TurnUsageChip',
  component: TurnUsageChip,
} satisfies Meta<typeof TurnUsageChip>

export default meta
type Story = StoryObj<typeof meta>

export const Collapsed: Story = { args: { turn: warmingTurn } }

/** Dock density drops the cost figure so the header still fits at 320px. */
export const Compact: Story = { args: { turn: warmingTurn, compact: true } }

/** Usage typically lands only on the final frame, so a live turn shows dashes. */
export const Streaming: Story = {
  args: {
    turn: {
      ...warmingTurn,
      steps: 0,
      stepUsage: [],
      totals: totals({
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        costUsd: 0,
        total: 0,
        reported: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          cost: 0,
        },
      }),
      streaming: true,
    },
  },
}

/**
 * A turn with nothing measured and nothing running renders NOTHING — sessions
 * written before usage was persisted look exactly as they do today.
 */
export const NullGuard: Story = {
  args: {
    turn: {
      ...warmingTurn,
      steps: 0,
      stepUsage: [],
      totals: totals({
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        costUsd: 0,
        total: 0,
        reported: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          reasoning: 0,
          cost: 0,
        },
      }),
      streaming: false,
    },
  },
}
