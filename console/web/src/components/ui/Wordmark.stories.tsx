import type { Meta, StoryObj } from '@storybook/react-vite'
import { Wordmark } from './Wordmark'

const meta = {
  title: 'UI/Wordmark',
  component: Wordmark,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Wordmark>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => (
    <div className="flex items-center gap-4">
      <Wordmark />
      <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
        three eye
      </span>
    </div>
  ),
}

export const InverseOnInk: Story = {
  name: 'inverse (on ink panel)',
  render: () => (
    <div className="flex items-center gap-4 bg-ink px-4 py-3">
      <Wordmark tone="inverse" />
      <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-bg">
        three eye
      </span>
    </div>
  ),
}
