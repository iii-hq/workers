import type { Meta, StoryObj } from '@storybook/react-vite'
import { StatusDot } from './StatusDot'

const meta = {
  title: 'UI/StatusDot',
  component: StatusDot,
  parameters: { layout: 'padded' },
  argTypes: {
    tone: { control: 'select', options: ['accent', 'alert', 'warn', 'ink'] },
    pulse: { control: 'boolean' },
  },
} satisfies Meta<typeof StatusDot>

export default meta
type Story = StoryObj<typeof meta>

export const LivePulse: Story = { args: { tone: 'accent', pulse: true } }

export const Tones: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-x-6 gap-y-3 font-mono text-[12px] text-ink-faint lowercase">
      <span className="flex items-center gap-2">
        <StatusDot tone="accent" pulse /> live
      </span>
      <span className="flex items-center gap-2">
        <StatusDot tone="accent" /> ok
      </span>
      <span className="flex items-center gap-2">
        <StatusDot tone="warn" /> warn
      </span>
      <span className="flex items-center gap-2">
        <StatusDot tone="alert" /> alert
      </span>
      <span className="flex items-center gap-2">
        <StatusDot tone="ink" /> idle
      </span>
    </div>
  ),
}
