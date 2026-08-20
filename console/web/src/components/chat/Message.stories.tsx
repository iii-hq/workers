import type { Meta, StoryObj } from '@storybook/react-vite'
import { registrationFromCall } from '@/components/trigger-activity/model'
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
  content: 'orders watch · called example::check_completion',
  trigger: {
    subscription_id: 'sub_00000000000000000000000000000001',
    trigger_id: '00000000-0000-4000-8000-000000000001',
    target: 'example::check_completion',
    label: 'orders watch',
    once: false,
    retired: false,
    fired_at: 1785948999879,
    payload: {
      event: {
        db: 'primary',
        table: 'orders',
        op: 'update',
        affected_rows: 1,
        at: 1785948999879,
      },
    },
  },
  createdAt: 1785948999879,
}

const triggerFiredOnce: SystemMessage = {
  id: 'e_trigfired_sub_story_1',
  role: 'system',
  kind: 'trigger-fired',
  tone: 'info',
  content: 'daily report · notified this chat · once consumed',
  trigger: {
    subscription_id: 'sub_story',
    trigger_id: 'trigger-story',
    trigger_type: 'cron',
    config: { expression: '0 30 9 * * *' },
    target: 'harness::send',
    label: 'daily report',
    once: true,
    retired: true,
    fires: 1,
    fired_at: 1_785_948_999_879,
    outcome: 'delivered',
    retirement_reason: 'once_consumed',
  },
  createdAt: 1_785_948_999_879,
}

const triggerNotification: UserMessage = {
  id: 'e_fire_sub_story_1',
  role: 'user',
  content: '[notification] daily report: {"report_id":"rpt-42","rows":128}',
  notification: true,
  triggerBindingId: 'sub_story',
  createdAt: 1_785_948_999_879,
}

const triggerManuallyRemoved: SystemMessage = {
  id: 'e_trigexpired_sub_manual',
  role: 'system',
  kind: 'trigger-fired',
  tone: 'info',
  content: 'orders watch · binding manually removed',
  trigger: {
    subscription_id: 'sub_manual',
    trigger_type: 'database::row-changed',
    config: { database: 'primary', table: 'orders' },
    target: 'orders::reindex',
    label: 'orders watch',
    once: false,
    retired: true,
    fires: 4,
    fired_at: 1_785_949_099_879,
    outcome: 'unregistered',
    retirement_reason: 'unregistered',
  },
  createdAt: 1_785_949_099_879,
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

export const TriggerFiredOnceConsumed: Story = {
  name: 'system, trigger fired · once consumed',
  args: {
    message: triggerFiredOnce,
    triggerNotification,
    registration: registrationFromCall({
      id: 'register-story',
      subscriptionId: 'sub_story',
      effectiveOnce: true,
      input: {
        trigger_type: 'cron',
        config: { expression: '0 30 9 * * *' },
        label: 'daily report',
      },
    }),
  },
}

export const TriggerBindingManuallyRemoved: Story = {
  name: 'system, trigger binding · manually removed',
  args: { message: triggerManuallyRemoved },
}
