import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ThoughtMessage as ThoughtType } from '@/types/chat'
import { ThoughtMessage } from './ThoughtMessage'

const thoughtBrief: ThoughtType = {
  id: 't1',
  role: 'thought',
  content:
    'this is a one-line clarification. trivial to resolve, no branching to consider.',
  durationMs: 800,
  createdAt: Date.now(),
}

const thoughtLong: ThoughtType = {
  id: 't2',
  role: 'thought',
  content:
    "restating the request in one line, then enumerating constraints. the user mentioned shape and direction but not scale, so i'll plan for the smaller end of the range and flag the bigger case as a follow-up.",
  durationMs: 2300,
  createdAt: Date.now(),
}

const thoughtStreaming: ThoughtType = {
  id: 't3',
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

export const BriefCollapsed: Story = {
  name: 'thought, briefly (collapsed)',
  args: { message: thoughtBrief },
}

export const LongCollapsed: Story = {
  name: 'thought, 2.3s (collapsed)',
  args: { message: thoughtLong },
}

export const Expanded: Story = {
  name: 'thought, expanded',
  args: { message: thoughtLong, defaultOpen: true },
}
