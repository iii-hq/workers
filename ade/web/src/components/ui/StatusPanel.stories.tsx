import type { Meta, StoryObj } from '@storybook/react-vite'
import { CircleAlert, CircleCheck, Info, TriangleAlert } from 'lucide-react'
import { StatusPanel } from './StatusPanel'

const meta = {
  title: 'UI/StatusPanel',
  component: StatusPanel,
  parameters: { layout: 'padded' },
  args: {
    headline: 'Worker reconnected',
    detail: 'Sessions resumed from the last checkpoint; no turns were lost.',
  },
  argTypes: {
    variant: {
      control: 'select',
      options: ['info', 'success', 'warn', 'alert'],
    },
  },
  decorators: [
    (Story) => (
      <div className="max-w-md">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof StatusPanel>

export default meta
type Story = StoryObj<typeof meta>

export const Neutral: Story = {
  args: { variant: 'info', icon: <Info size={18} /> },
}

export const Success: Story = {
  args: { variant: 'success', icon: <CircleCheck size={18} /> },
}

export const Warn: Story = {
  args: {
    variant: 'warn',
    icon: <TriangleAlert size={18} />,
    headline: 'Engine version drift',
    detail: 'The console runs 0.11.4; worker browser reports 0.11.2.',
  },
}

export const Alert: Story = {
  args: {
    variant: 'alert',
    icon: <CircleAlert size={18} />,
    headline: 'Worker crashed',
    detail: 'browser exited with code 137 (out of memory).',
  },
}

/** No icon and no detail: a one-line notice. */
export const HeadlineOnly: Story = {
  args: { variant: 'info', detail: undefined },
}
