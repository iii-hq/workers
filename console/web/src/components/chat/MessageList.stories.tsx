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

function agentLoop(steps: number): Message[] {
  const messages: Message[] = [
    {
      id: 'u-0',
      createdAt: 1,
      role: 'user',
      content:
        'Port the python client to rust, keep the public API, and prove it with the existing fixtures.',
    },
  ]
  let clock = 2
  for (let index = 0; index < steps; index += 1) {
    messages.push({
      id: `f-${index}`,
      createdAt: clock++,
      role: 'function-trigger',
      functionId: index % 2 === 0 ? 'coder::read-file' : 'shell::exec',
      input: { path: `src/step-${index}.rs` },
      output: { ok: true },
      durationMs: 80 + index,
    })
    messages.push({
      id: `a-${index}`,
      createdAt: clock++,
      role: 'assistant',
      model: 'anthropic::claude-haiku-4-5-20251001',
      mode: 'agent',
      content: `Step ${index + 1}: ${index % 5 === 4 ? 'tests pass for this module, moving on.' : 'reading the module and sketching the rust signature before writing it.'}`,
      stopReason: 'end',
      streaming: index === steps - 1,
    })
  }
  return messages
}

export const AgentLoopWithTurnRail: Story = {
  name: 'single prompt, long agent loop',
  args: { messages: agentLoop(30) },
}

export const ShortSessionNoRail: Story = {
  name: 'short session, no rail',
  args: { messages: longSession(3) },
}
