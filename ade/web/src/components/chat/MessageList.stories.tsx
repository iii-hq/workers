import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type { FunctionCallMessage, Message, ThoughtMessage } from '@/types/chat'
import { MessageList } from './MessageList'

const completedCall = (
  id: string,
  description: string,
): FunctionCallMessage => ({
  id,
  role: 'function-call',
  functionId: 'directory::index',
  description,
  input: { path: '/repo/src' },
  output: { ok: true },
  durationMs: 120,
  createdAt: 0,
})

const completedThought = (id: string): ThoughtMessage => ({
  id,
  role: 'thought',
  content: `Reasoning step ${id}`,
  durationMs: 180,
  streaming: false,
  createdAt: 0,
})

const sevenCompletedCalls: Message[] = [
  completedCall('call-1', 'Localizando ferramentas para inspecionar o código'),
  completedCall('call-2', 'Listando a estrutura principal do projeto'),
  completedCall('call-3', 'Encontrando os módulos relacionados à tasklist'),
  completedCall('call-4', 'Inspecionando os tipos usados pela interface'),
  completedCall('call-5', 'Revisando o fluxo de atualização da tasklist'),
  completedCall('call-6', 'Conferindo os testes existentes'),
  completedCall('call-7', 'Ler arquivo principal index.ts do tasklist'),
]

const callsAcrossCompletedThoughts: Message[] = [
  completedCall(
    'phase-call-1',
    'Localizando ferramentas para inspecionar o código',
  ),
  completedThought('phase-thought-1'),
  completedCall('phase-call-2', 'Listando a estrutura principal do projeto'),
  completedCall('phase-call-3', 'Inspecionando os módulos encontrados'),
  completedThought('phase-thought-2'),
  completedCall('phase-call-4', 'Lendo os arquivos selecionados'),
]

function ThinkingToCallHandoffDemo({ priorCall }: { priorCall: boolean }) {
  const [settled, setSettled] = useState(false)
  const thought = completedThought('handoff-thought')
  const prior = completedCall('handoff-call-1', 'Inspecionando a implementação')
  const next = completedCall(
    priorCall ? 'handoff-call-2' : 'handoff-call-1',
    'Validando o próximo arquivo',
  )
  const messages: Message[] = settled
    ? priorCall
      ? [prior, thought, next]
      : [thought, next]
    : priorCall
      ? [prior, { ...thought, streaming: true }]
      : [{ ...thought, streaming: true }]

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex justify-end px-4 pt-3">
        <button
          type="button"
          className="rounded-md border border-rule-2 px-3 py-1.5 font-mono text-xs text-ink-faint hover:text-ink"
          onClick={() => setSettled((value) => !value)}
        >
          {settled ? 'Replay thinking' : 'Complete thinking'}
        </button>
      </div>
      <div className="flex min-h-0 flex-1">
        <MessageList messages={messages} transcriptHydrated />
      </div>
    </div>
  )
}

const meta = {
  title: 'Chat/MessageList',
  component: MessageList,
  parameters: { layout: 'fullscreen' },
  decorators: [
    (Story) => (
      <div className="flex h-[360px] min-h-0 bg-bg">
        <Story />
      </div>
    ),
  ],
  args: {
    messages: [],
    transcriptHydrated: false,
  },
} satisfies Meta<typeof MessageList>

export default meta
type Story = StoryObj<typeof meta>

export const SevenCompletedCallsCollapsed: Story = {
  name: 'seven completed calls · collapsed',
  args: { messages: sevenCompletedCalls },
}

export const ConsecutiveCallsAcrossSettledThoughts: Story = {
  name: 'consecutive calls across settled thoughts',
  args: { messages: callsAcrossCompletedThoughts },
}

export const ThinkingToCallHandoff: Story = {
  name: 'thinking → next call handoff',
  render: () => <ThinkingToCallHandoffDemo priorCall />,
}

export const ThinkingToFirstCallHandoff: Story = {
  name: 'thinking → first call handoff',
  render: () => <ThinkingToCallHandoffDemo priorCall={false} />,
}
