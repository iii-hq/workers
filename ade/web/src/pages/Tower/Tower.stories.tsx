import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { TowerScreen, type TowerScreenProps } from '.'
import { towerFixture } from './fixtures'

function FixtureScreen(props: Partial<TowerScreenProps>) {
  const [scope, setScope] = useState('all')
  return (
    <div style={{ height: '100vh' }}>
      <TowerScreen
        data={towerFixture}
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
export const Runs: Story = { args: { initialView: 'runs' } }
export const NeedsYouPlaceholder: Story = { args: { initialView: 'needs-you' } }
export const Loading: Story = {
  args: {
    data: {
      ...towerFixture,
      runs: [],
      items: [],
      prs: [],
      releases: [],
      loading: { board: true, sessions: true, needsYou: true, prs: true },
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
