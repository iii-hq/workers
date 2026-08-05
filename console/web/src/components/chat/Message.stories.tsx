import type { Meta, StoryObj } from '@storybook/react-vite'
import type {
  AssistantMessage,
  Attachment,
  SystemMessage,
  UserMessage,
} from '@/types/chat'
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
  mode: 'agent',
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

const triggerFiredCall: SystemMessage = {
  id: 's_trig1',
  role: 'system',
  kind: 'trigger-fired',
  tone: 'info',
  content: 'courier watch · called receiving::check_completion',
  trigger: {
    subscription_id: 'sub_9aea3d241f094c8693538ff70f86ab80',
    trigger_id: 'b74ff278-a8ab-4d63-9db2-a084fec3655a',
    target: 'receiving::check_completion',
    label: 'courier watch',
    once: false,
    retired: false,
    fired_at: 1785948999879,
    payload: {
      event: {
        db: 'primary',
        table: 'courier_status',
        op: 'update',
        affected_rows: 1,
        at: 1785948999879,
      },
    },
  },
  createdAt: 1785948999879,
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

export const TriggerFiredWithPayload: Story = {
  name: 'system, trigger fired · ƒ-call with payload',
  args: { message: triggerFiredCall },
}
