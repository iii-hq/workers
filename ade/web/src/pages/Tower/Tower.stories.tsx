import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { TowerScreen, type TowerScreenProps } from '.'
import { towerFixture, towerSummaryFixture } from './fixtures'
import type { TowerActions } from './NeedsYou'

const fixtureActions: TowerActions = {
  async call<T>(functionId: string, payload?: Record<string, unknown>) {
    const entry = towerFixture.questions.find(
      (row) => row.questionId === payload?.question_id,
    )?.harness
    return (
      functionId === 'harness::ask::list'
        ? {
            entries: towerFixture.questions.flatMap((row) =>
              row.harness ? [row.harness] : [],
            ),
          }
        : entry
          ? {
              ...entry,
              state: payload?.dismissed ? 'dismissed' : 'answered',
              by: { channel: 'tower', user: 'user' },
              answers: payload?.answers,
            }
          : { resolved: true }
    ) as T
  },
}

function FixtureScreen(props: Partial<TowerScreenProps>) {
  const [scope, setScope] = useState('all')
  return (
    <div style={{ height: '100vh' }}>
      <TowerScreen
        data={towerFixture}
        iii={fixtureActions}
        scope={scope}
        onScopeChange={setScope}
        {...props}
      />
    </div>
  )
}
const meta = {
  title: 'Pages/Control tower',
  component: FixtureScreen,
  parameters: { layout: 'fullscreen' },
} satisfies Meta<typeof FixtureScreen>
export default meta
type Story = StoryObj<typeof meta>
export const Overview: Story = {}
export const FreshSummary: Story = {
  args: {
    summary: {
      result: towerSummaryFixture,
      loading: false,
      error: null,
      refresh: () => {},
      now: Date.now(),
    },
  },
}
export const RunsWithSummary: Story = {
  args: { ...FreshSummary.args, initialView: 'runs' },
}
export const Runs: Story = { args: { initialView: 'runs' } }
export const NeedsYou: Story = { args: { initialView: 'needs-you' } }
export const Loading: Story = {
  args: {
    data: {
      ...towerFixture,
      runs: [],
      items: [],
      prs: [],
      releases: [],
      loading: {
        board: true,
        sessions: true,
        needsYou: true,
        questions: true,
        prs: true,
      },
    },
  },
}
export const GithubUnreachable: Story = {
  args: {
    data: {
      ...towerFixture,
      prs: [],
      errors: { ...towerFixture.errors, prs: 'GitHub unavailable' },
    },
  },
}
export const NoRepositories: Story = {
  args: { data: { ...towerFixture, repositories: [], prs: [] } },
}
export const StaleSummary: Story = {
  args: { summaryCheckedAt: Date.now() - 42 * 60_000 },
}
export const EmptyProject: Story = {
  args: {
    scope: 'empty',
    data: {
      ...towerFixture,
      board: { columns: [], doneStatus: 'done', idPrefix: 'KAN' },
      tickets: [],
      runs: [],
      items: [],
      prs: [],
      releases: [],
      repositories: [],
    },
  },
}
export const RunsLoading: Story = {
  args: {
    initialView: 'runs',
    data: {
      ...towerFixture,
      runs: [],
      loading: { ...towerFixture.loading, sessions: true },
    },
  },
}
export const RunsFailed: Story = {
  args: {
    initialView: 'runs',
    data: {
      ...towerFixture,
      runs: [],
      errors: {
        ...towerFixture.errors,
        sessions:
          'session-manager did not answer. Needs you and the board are unaffected.',
      },
    },
  },
}

export const Assumptions: Story = { args: { initialView: 'assumptions' } }
export const Decided: Story = { args: { initialView: 'decided' } }

export const HeldCall: Story = {
  args: { initialView: 'needs-you', initialSelectedId: 'hold' },
}
export const ExpiredQuestion: Story = {
  args: { initialView: 'needs-you', initialSelectedId: 'verify' },
}
export const NeedsYouEmpty: Story = {
  args: { initialView: 'needs-you', data: { ...towerFixture, items: [] } },
}
export const NeedsYouLoading: Story = {
  args: {
    initialView: 'needs-you',
    data: {
      ...towerFixture,
      items: [],
      loading: { ...towerFixture.loading, needsYou: true },
    },
  },
}
export const NeedsYouSourceDown: Story = {
  args: {
    initialView: 'needs-you',
    data: {
      ...towerFixture,
      errors: {
        ...towerFixture.errors,
        needsYou:
          'approval-gate is unavailable. Held calls will appear when it returns.',
      },
    },
  },
}
