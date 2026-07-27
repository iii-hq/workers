import type { Meta, StoryObj } from '@storybook/react-vite'
import type { SessionUsage, UsageTotals } from '@/lib/session-usage'
import { SessionMetricsPanel } from './SessionMetricsPanel'

function totals(over: Partial<UsageTotals> = {}): UsageTotals {
  const base: UsageTotals = {
    input: 412_908,
    output: 18_204,
    cacheRead: 388_110,
    cacheWrite: 12_400,
    reasoning: 0,
    costUsd: 0.4821,
    reported: {
      input: 39,
      output: 39,
      cacheRead: 39,
      cacheWrite: 39,
      // anthropic never reports reasoning — this must render `—`, not `0`.
      reasoning: 0,
      cost: 39,
    },
    total: 431_112,
  }
  return { ...base, ...over }
}

const emptyTotals = totals({
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  reasoning: 0,
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
})

function turn(id: string, i: number, streaming = false) {
  return {
    turnId: id,
    steps: 3,
    stepUsage: [],
    totals: totals({ total: 62_100 * i, costUsd: 0.031 * i }),
    functionCalls: 4,
    functionCallErrors: i === 2 ? 1 : 0,
    startedAt: 0,
    endedAt: 62_000 * i,
    durationMs: 62_000 * i,
    streaming,
  }
}

const full: SessionUsage = {
  totals: totals(),
  turns: [turn('t_9f1a', 1), turn('t_a02b', 2), turn('t_b17c', 3)],
  steps: 39,
  stepsMissingUsage: 3,
  functionCalls: 61,
  functionCallErrors: 3,
  startedAt: 0,
  endedAt: 724_000,
  durationMs: 724_000,
  lastCall: { usage: { input: 96_410, output: 288 }, at: 724_000 },
}

const meta = {
  title: 'metrics/SessionMetricsPanel',
  component: SessionMetricsPanel,
  args: {
    contextEstimate: 84_200,
    contextWindow: 200_000,
    showTurnChips: true,
    onToggleTurnChips: () => {},
    onViewTraces: () => {},
    onRetryTree: () => {},
  },
  decorators: [
    (Story) => (
      <div className="max-w-2xl p-6 font-mono">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionMetricsPanel>

export default meta
type Story = StoryObj<typeof meta>

export const Full: Story = {
  args: {
    usage: full,
    tree: {
      status: 'ok',
      metrics: {
        root_session_id: 's_3f2a',
        complete: true,
        totals: {
          sessions: 3,
          turns: 52,
          function_calls: 88,
          function_call_errors: 4,
          input_tokens: 610_400,
          output_tokens: 24_900,
          cache_read_tokens: 502_100,
          cache_write_tokens: 18_200,
          reasoning_tokens: null,
          cost_usd: 0.7412,
        },
        by_session: [
          {
            session_id: 's_3f2a',
            depth: 0,
            sessions: 1,
            turns: 39,
            function_calls: 61,
            function_call_errors: 3,
            input_tokens: 412_908,
            output_tokens: 18_204,
          },
          {
            session_id: 's_child1',
            parent_session_id: 's_3f2a',
            depth: 1,
            sessions: 1,
            turns: 13,
            function_calls: 27,
            function_call_errors: 1,
            input_tokens: 197_492,
            output_tokens: 6_696,
          },
        ],
        traces: {
          trace_count: 39,
          span_count: 412,
          error_span_count: 2,
          duration_ms: 724_000,
        },
      },
    },
  },
}

/** Before the backend persisted usage — exact rows dash out, counted rows work. */
export const NoUsage: Story = {
  args: {
    usage: {
      ...full,
      totals: emptyTotals,
      stepsMissingUsage: 39,
      turns: [],
      lastCall: undefined,
    },
    tree: { status: 'unavailable' },
  },
}

/** codex reports no cache_write; anthropic reports no reasoning. */
export const PartialProvider: Story = {
  args: {
    usage: {
      ...full,
      totals: totals({
        cacheWrite: 0,
        reported: {
          input: 39,
          output: 39,
          cacheRead: 39,
          cacheWrite: 0,
          reasoning: 0,
          cost: 0,
        },
      }),
    },
    tree: { status: 'unavailable' },
  },
}

export const Streaming: Story = {
  args: {
    usage: { ...full, turns: [turn('t_9f1a', 1), turn('t_live', 2, true)] },
    tree: { status: 'incomplete' },
  },
}

/** The common case for an active chat — never render the zeroed payload. */
export const TreeUnavailable: Story = {
  args: { usage: full, tree: { status: 'incomplete' } },
}
