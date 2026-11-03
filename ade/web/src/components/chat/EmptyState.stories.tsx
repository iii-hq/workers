import type { Meta, StoryObj } from '@storybook/react-vite'
import { fn } from 'storybook/test'
import { EmptyState } from './EmptyState'

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
      <div className="@container flex h-[680px] w-full flex-col bg-bg">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof EmptyState>

export default meta
type Story = StoryObj<typeof meta>

export const Ready: Story = {
  name: 'ready (harness + provider)',
  args: { variant: 'ready' },
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
  args: { variant: 'installing' },
}

export const InstallFailed: Story = {
  name: 'install failed',
  args: { variant: 'install-failed', errorMessage: 'registry unreachable' },
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
