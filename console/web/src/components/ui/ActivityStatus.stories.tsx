import type { Meta, StoryObj } from '@storybook/react-vite'
import {
  Activity,
  BrainCircuit,
  CircleAlert,
  CircleOff,
  LoaderCircle,
} from 'lucide-react'
import { ActivityStatus } from './ActivityStatus'

const meta = {
  title: 'UI/ActivityStatus',
  component: ActivityStatus,
  args: {
    label: 'Active',
    detail: 'Active for 5m',
    icon: Activity,
    tone: 'positive',
  },
  parameters: { layout: 'padded' },
} satisfies Meta<typeof ActivityStatus>

export default meta
type Story = StoryObj<typeof meta>

export const Active: Story = {}

export const States: Story = {
  render: () => (
    <div className="grid max-w-sm gap-4">
      <ActivityStatus
        label="Active"
        detail="Active for 5m"
        icon={Activity}
        tone="positive"
      />
      <ActivityStatus
        label="Working"
        detail="Working for 2m"
        icon={LoaderCircle}
        motion="spin"
      />
      <ActivityStatus
        label="Thinking"
        detail="Thinking now"
        icon={BrainCircuit}
        motion="pulse"
        tone="accent"
      />
      <ActivityStatus
        label="Inactive"
        detail="No longer listening"
        icon={CircleOff}
      />
      <ActivityStatus
        label="Needs attention"
        detail="Delivery failed"
        icon={CircleAlert}
        tone="danger"
      />
    </div>
  ),
}
