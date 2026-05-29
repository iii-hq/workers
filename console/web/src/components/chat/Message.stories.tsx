import type { Meta, StoryObj } from '@storybook/react-vite'
import type { AssistantMessage, Attachment, UserMessage } from '@/types/chat'
import { Message } from './Message'

const sampleAttachments: Attachment[] = [
  { id: 'a1', name: 'spec.md', size: 4_312, type: 'text/markdown' },
  { id: 'a2', name: 'screenshot.png', size: 281_402, type: 'image/png' },
]

const userPlain: UserMessage = {
  id: 'u1',
  role: 'user',
  content: 'how do i wire @fn(engine::echo) into the agent loop?',
  createdAt: Date.now(),
}

const userWithAttachments: UserMessage = {
  id: 'u2',
  role: 'user',
  content: 'please review the attached spec and screenshot.',
  attachments: sampleAttachments,
  createdAt: Date.now(),
}

const assistantComplete: AssistantMessage = {
  id: 'a1',
  role: 'assistant',
  model: 'anthropic::claude-opus-4-7',
  mode: 'plan',
  content: `## the plan

three small steps, each independently revertible:

1. register the trigger on engine startup.
2. wire the callback through the dispatcher.
3. add a smoke test that exercises the happy path.

\`\`\`ts
engine.on('echo', (input) => ({ text: input.text }))
\`\`\`

> a one-liner you can keep in your back pocket.`,
  createdAt: Date.now(),
}

const assistantStreaming: AssistantMessage = {
  id: 'a2',
  role: 'assistant',
  model: 'openai::gpt-5',
  mode: 'ask',
  content:
    'sure — a btree-backed index gives you both `O(log n)` lookups and cheap range',
  streaming: true,
  createdAt: Date.now(),
}

const assistantThinking: AssistantMessage = {
  id: 'a3',
  role: 'assistant',
  model: 'anthropic::claude-opus-4-7',
  mode: 'agent',
  content: '',
  streaming: true,
  createdAt: Date.now(),
}

const meta = {
  title: 'Chat/Message',
  component: Message,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof Message>

export default meta
type Story = StoryObj<typeof meta>

export const UserPlain: Story = {
  name: 'user, plain',
  args: { message: userPlain },
}

export const UserWithAttachments: Story = {
  name: 'user, with attachments',
  args: { message: userWithAttachments },
}

export const AssistantComplete: Story = {
  name: 'assistant, complete',
  args: { message: assistantComplete },
}

export const AssistantStreaming: Story = {
  name: 'assistant, streaming',
  args: { message: assistantStreaming },
}

export const AssistantThinking: Story = {
  name: 'assistant, thinking (no content yet)',
  args: { message: assistantThinking },
}
