import type { Meta, StoryObj } from '@storybook/react-vite'
import type { Message } from '@/types/chat'
import { MessageList } from './MessageList'

const TOPICS = [
  'Why does the router drop the first model after a reconnect?',
  'Add a retry budget to the queue worker and show it in the console.',
  'Summarize the failing harness e2e and propose a fix.',
  'Which workers register functions with an empty request schema?',
  'Draft the release notes for shell 0.12.',
  'Port the database introspection to the editor worker.',
  'Explain the ask-mode policy cap in two paragraphs.',
  'Find every call site of configuration::set in the console.',
]

function longSession(turns: number): Message[] {
  const messages: Message[] = []
  let clock = 1
  for (let index = 0; index < turns; index += 1) {
    const topic = TOPICS[index % TOPICS.length]
    messages.push({
      id: `u-${index}`,
      createdAt: clock++,
      role: 'user',
      content: `${topic} (turn ${index + 1})`,
    })
    const calls = index % 3
    for (let call = 0; call < calls; call += 1) {
      messages.push({
        id: `f-${index}-${call}`,
        createdAt: clock++,
        role: 'function-trigger',
        functionId: call === 0 ? 'shell::exec' : 'coder::read-file',
        input: { path: `src/module-${index}.rs` },
        output: { ok: true },
        durationMs: 120 + call * 40,
      })
    }
    const failed = index % 11 === 7
    messages.push({
      id: `a-${index}`,
      createdAt: clock++,
      role: 'assistant',
      model: 'anthropic::claude-sonnet-4-6',
      mode: 'agent',
      content: failed
        ? 'The provider rejected the request before any tokens streamed.'
        : `Here is what I found for "${topic.toLowerCase()}". ${'The relevant code path runs through the router registry, then the provider adapter, then the stream writer. '.repeat(2 + (index % 4))}`,
      stopReason: failed ? 'error' : 'end',
      streaming: index === turns - 1,
    })
  }
  return messages
}

const meta = {
  title: 'Chat/MessageList',
  component: MessageList,
  parameters: { layout: 'fullscreen' },
  decorators: [
    (Story) => (
      <div className="flex h-screen w-full bg-bg">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MessageList>

export default meta
type Story = StoryObj<typeof meta>

export const LongSessionWithTurnRail: Story = {
  name: 'long session, turn rail',
  args: { messages: longSession(40) },
}

export const ShortSessionNoRail: Story = {
  name: 'short session, no rail',
  args: { messages: longSession(3) },
}
