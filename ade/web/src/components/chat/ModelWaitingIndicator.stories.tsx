import type { Meta, StoryObj } from '@storybook/react-vite'
import { ModelWaitingIndicator } from './ModelWaitingIndicator'

const meta = {
  title: 'Chat/ModelWaitingIndicator',
  component: ModelWaitingIndicator,
  args: {
    label: 'dispatching anthropic::claude-sonnet-4-5',
  },
  parameters: { layout: 'padded' },
} satisfies Meta<typeof ModelWaitingIndicator>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const Inactive: Story = {
  args: { active: false },
}

export const Narrow: Story = {
  decorators: [
    (Story) => (
      <div className="w-64">
        <Story />
      </div>
    ),
  ],
}
