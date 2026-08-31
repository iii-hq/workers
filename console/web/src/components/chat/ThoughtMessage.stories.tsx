import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ThoughtMessage as ThoughtType } from '@/types/chat'
import { ThoughtMessage } from './ThoughtMessage'

const thoughtStreaming: ThoughtType = {
  id: 't1',
  role: 'thought',
  content: 'enumerating the constraints…',
  durationMs: 0,
  streaming: true,
  createdAt: Date.now(),
}

const meta = {
  title: 'Chat/ThoughtMessage',
  component: ThoughtMessage,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof ThoughtMessage>

export default meta
type Story = StoryObj<typeof meta>

export const Streaming: Story = {
  name: 'thought, streaming',
  args: { message: thoughtStreaming },
}

export const Long: Story = {
  name: 'thought, five-line live tail',
  args: {
    message: {
      ...thoughtStreaming,
      content: Array.from(
        { length: 12 },
        (_, index) => `${index + 1}. inspecting another constraint…`,
      ).join('\n'),
    },
  },
}

export const Settling: Story = {
  name: 'thought, settling',
  args: {
    message: { ...thoughtStreaming, streaming: false, durationMs: 1250 },
    settling: true,
  },
}
