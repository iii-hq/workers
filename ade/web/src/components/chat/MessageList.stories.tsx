import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import type {
  FunctionTriggerMessage,
  Message,
  ThoughtMessage,
} from '@/types/chat'
import { MessageList } from './MessageList'

const completedTrigger = (
  id: string,
  description: string,
): FunctionTriggerMessage => ({
  id,
  role: 'function-trigger',
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

const sevenCompletedTriggers: Message[] = [
  completedTrigger(
    'call-1',
    'Localizando ferramentas para inspecionar o código',
  ),
  completedTrigger('call-2', 'Listando a estrutura principal do projeto'),
  completedTrigger('call-3', 'Encontrando os módulos relacionados à tasklist'),
  completedTrigger('call-4', 'Inspecionando os tipos usados pela interface'),
  completedTrigger('call-5', 'Revisando o fluxo de atualização da tasklist'),
  completedTrigger('call-6', 'Conferindo os testes existentes'),
  completedTrigger('call-7', 'Ler arquivo principal index.ts do tasklist'),
]

const triggersAcrossCompletedThoughts: Message[] = [
  completedTrigger(
    'phase-call-1',
    'Localizando ferramentas para inspecionar o código',
  ),
  completedThought('phase-thought-1'),
  completedTrigger('phase-call-2', 'Listando a estrutura principal do projeto'),
  completedTrigger('phase-call-3', 'Inspecionando os módulos encontrados'),
  completedThought('phase-thought-2'),
  completedTrigger('phase-call-4', 'Lendo os arquivos selecionados'),
]

function ThinkingToTriggerHandoffDemo({
  priorTrigger,
}: {
  priorTrigger: boolean
}) {
  const [settled, setSettled] = useState(false)
  const thought = completedThought('handoff-thought')
  const prior = completedTrigger(
    'handoff-call-1',
    'Inspecionando a implementação',
  )
  const next = completedTrigger(
    priorTrigger ? 'handoff-call-2' : 'handoff-call-1',
    'Validando o próximo arquivo',
  )
  const messages: Message[] = settled
    ? priorTrigger
      ? [prior, thought, next]
      : [thought, next]
    : priorTrigger
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

export const SevenCompletedTriggersCollapsed: Story = {
  name: 'seven completed triggers · collapsed',
  args: { messages: sevenCompletedTriggers },
}

export const ConsecutiveTriggersAcrossSettledThoughts: Story = {
  name: 'consecutive triggers across settled thoughts',
  args: { messages: triggersAcrossCompletedThoughts },
}

export const ThinkingToTriggerHandoff: Story = {
  name: 'thinking → next trigger handoff',
  render: () => <ThinkingToTriggerHandoffDemo priorTrigger />,
}

export const ThinkingToFirstTriggerHandoff: Story = {
  name: 'thinking → first trigger handoff',
  render: () => <ThinkingToTriggerHandoffDemo priorTrigger={false} />,
}
