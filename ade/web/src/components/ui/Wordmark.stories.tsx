import type { Meta, StoryObj } from '@storybook/react-vite'
import { Eyebrow } from '@/components/ui/Eyebrow'
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
      <Eyebrow size="lg">three eye</Eyebrow>
    </div>
  ),
}

export const Inset: Story = {
  render: () => (
    <div className="flex items-center justify-center bg-bg p-8">
      <Wordmark appearance="inset" />
    </div>
  ),
}

export const InverseOnInk: Story = {
  name: 'inverse (on ink panel)',
  render: () => (
    <div className="flex items-center gap-4 bg-ink px-4 py-3">
      <Wordmark tone="inverse" />
      <Eyebrow size="lg" className="text-bg">
        three eye
      </Eyebrow>
    </div>
  ),
}
