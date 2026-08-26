import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { fn } from 'storybook/test'
import type { InstallStage } from '@/hooks/use-harness-status'
import { EmptyState, type EmptyStateProps } from './EmptyState'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SkillSelection,
  type SystemPromptState,
} from './system-prompt-selection'

/** Mid-download progress for the live install console. */
const installingStages: InstallStage[] = [
  { stage: 'started', worker: 'harness', at: 1 },
  { stage: 'downloading', worker: 'harness', progress: 0.62, at: 2 },
]

/** A run that downloaded, then failed at the shim step. */
const failedStages: InstallStage[] = [
  { stage: 'started', worker: 'harness', at: 1 },
  { stage: 'downloading', worker: 'harness', progress: 1, at: 2 },
  { stage: 'downloaded', worker: 'harness', at: 3 },
  {
    stage: 'failed',
    worker: 'harness',
    error: { code: 'W900', message: 'registry unreachable' },
    at: 4,
  },
]

// EmptyState is `flex-1` and centers itself, so give it a tall flex column to
// fill — mirrors how it sits inside the chat surface.
const meta = {
  title: 'Chat/EmptyState',
  component: EmptyState,
  parameters: { layout: 'fullscreen' },
  args: {
    density: 'route',
    onInstallHarness: fn(),
    onRetryInstall: fn(),
    onConfigureProvider: fn(),
  },
  decorators: [
    (Story) => (
      <div className="flex h-[680px] w-full flex-col bg-bg">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof EmptyState>

export default meta
type Story = StoryObj<typeof meta>

function ConfiguredReadyStory(args: EmptyStateProps) {
  const [systemPrompt, setSystemPrompt] = useState<SystemPromptState>(
    args.systemPrompt ?? DEFAULT_SYSTEM_PROMPT_STATE,
  )
  const [skills, setSkills] = useState<SkillSelection>(args.skills)

  return (
    <EmptyState
      {...args}
      systemPrompt={systemPrompt}
      onSystemPromptChange={setSystemPrompt}
      skills={skills}
      onSkillsChange={setSkills}
    />
  )
}

export const Ready: Story = {
  name: 'ready (harness + provider)',
  args: {
    variant: 'ready',
    workingDir: '/Users/sergio/Documents/workspaces/iii/workers',
    defaultWorkingDir: '/Users/sergio/Documents/workspaces/iii/workers',
    onWorkingDirChange: fn(),
    systemPrompt: DEFAULT_SYSTEM_PROMPT_STATE,
    onSystemPromptChange: fn(),
    onSkillsChange: fn(),
  },
}

export const ReadyConfigured: Story = {
  name: 'ready (prompt + selected skills)',
  render: (args) => <ConfiguredReadyStory {...args} />,
  args: {
    variant: 'ready',
    workingDir: '/Users/sergio/Documents/workspaces/iii/workers',
    defaultWorkingDir: '/Users/sergio/Documents/workspaces/iii/workers',
    onWorkingDirChange: fn(),
    systemPrompt: {
      ...DEFAULT_SYSTEM_PROMPT_STATE,
      choice: { named: 'reviewer' },
    },
    onSystemPromptChange: fn(),
    skills: ['design', 'linear-workflow', 'canonicalize-tailwind'],
    onSkillsChange: fn(),
  },
}

export const ReadyWithoutProject: Story = {
  name: 'ready (project loading)',
  args: {
    variant: 'ready',
    workingDir: null,
    onWorkingDirChange: fn(),
    systemPrompt: DEFAULT_SYSTEM_PROMPT_STATE,
    onSystemPromptChange: fn(),
    onSkillsChange: fn(),
  },
}

export const NoProvider: Story = {
  name: 'no provider configured',
  args: { variant: 'no-provider' },
}

export const NoHarness: Story = {
  name: 'harness not installed',
  args: { variant: 'no-harness' },
}

export const Installing: Story = {
  name: 'installing (console)',
  args: { variant: 'installing', stages: installingStages },
}

export const InstallingNoEvents: Story = {
  name: 'installing (no progress events)',
  args: { variant: 'installing', stages: [] },
}

export const InstallFailed: Story = {
  name: 'install failed',
  args: { variant: 'install-failed', stages: failedStages },
}

export const Dock: Story = {
  name: 'harness not installed (dock density)',
  args: { variant: 'no-harness', density: 'dock' },
  decorators: [
    (Story) => (
      <div className="flex h-[680px] w-[420px] flex-col bg-bg">
        <Story />
      </div>
    ),
  ],
}
