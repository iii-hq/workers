import type { Meta, StoryObj } from '@storybook/react-vite'
import type { MemoryItem } from '@/lib/memory'
import { parseMemoryNote } from '@/lib/memory-note'
import type { ModelNote, SystemMessage } from '@/types/chat'
import { MemoryChip, MemoryChipPanel } from './MemoryChip'
import { ModelNoteMarker } from './ModelNoteMarker'
import './conversation-design.css'

const facts = [
  'Testar juntos os PRs #83 e #84 na stack local não exige fazer merge deles; o merge só é necessário para entregar a funcionalidade na main.',
  'O PR #79 foi mergeado em main no commit 1199f272; a correção do job Postgres tests (tabela ausente visit_logbook_evidence) havia sido verificada localmente no commit b0fe7be0, com 344 testes focados e 1.850 testes completos aprovados sem falhas, 302 pulados.',
  'O PR #84 ("Botão \'Criar rota da semana\' no painel") foi aberto para main a partir da branch workflow/botao-rota-semanal-147569-g3, sem merge, documentando os 56 testes aprovados, a dependência da API do PR #83 e a pendência de foco do teclado a corrigir antes do merge.',
  'O PR #81 foi mergeado em main, mudando apenas o job admin do CI para usar os runners self-hosted do Dokploy, enquanto os outros três jobs (test, auth-e2e, postgres) continuam em ubuntu-24.04.',
  'O PR #82 foi mergeado em main no commit 176e4d78, movendo os jobs admin, test e Postgres tests para os runners self-hosted do Dokploy; apenas o Security regression E2E continua hospedado pelo GitHub por isolamento de teste.',
]
const dates = [
  '2026-11-23',
  '2026-11-23',
  '2026-11-23',
  '2026-11-21',
  '2026-11-21',
]

const recall = `<memory bank="segralim-d86496">
Relevant remembered memories ("the user" is the person you are talking to; recalled 2026-11-23 Monday; each dated when last stated; verify anything surprising):
${facts.map((f, i) => `- ${f} (${dates[i]})`).join('\n')}
Standing instructions — apply every one to this reply:
- Responda em português.
- Nunca faça merge sem aprovação.
</memory>`

const rulesUpdate = `<memory update="rules">
The memory rules changed. This block replaces the memory section of the system prompt and any earlier update:
- Responda em português.
- Nunca faça merge sem aprovação.
</memory>`

function noteProps(
  id: string,
  text: string,
): { message: SystemMessage; note: ModelNote } {
  const memory = parseMemoryNote(text)
  const note: ModelNote = {
    label: 'memory',
    text,
    ...(memory ? { memory } : {}),
  }
  return {
    message: {
      id,
      role: 'system',
      kind: 'model-note',
      tone: 'info',
      content: 'Note to the model — memory',
      note,
      createdAt: 1,
    },
    note,
  }
}

const loaded: MemoryItem[] = facts.map((text, i) => ({
  id: `m${i}`,
  text,
  entities: [],
  tags: [],
  confidence: 'extracted',
  corroboration: 0,
  pinned: i === 1,
  created_at: Date.parse(`${dates[i]}T12:00:00`),
  updated_at: Date.parse(`${dates[i]}T12:00:00`),
  stated_at: Date.parse(`${dates[i]}T12:00:00`),
  revision: 0,
}))

function Transcript({ children }: { children: React.ReactNode }) {
  return (
    <section className="conversation-surface @container min-h-dvh bg-bg p-6 text-ink">
      <div className="mx-auto flex max-w-[760px] flex-col gap-6">
        {children}
      </div>
    </section>
  )
}

function Header() {
  return (
    <header className="chat-agent-header flex flex-wrap items-center font-sans">
      <span className="chat-agent-mark" aria-hidden="true" />
      <span className="chat-agent-name">Workflow</span>
      <span className="chat-agent-model">codex/gpt-6.2-astra</span>
      <MemoryChip
        memory={{
          bank: 'segralim-d86496',
          memories: 5,
          memoryIds: loaded.map((m) => m.id),
          memoryBanks: [],
          semantic: true,
        }}
      />
    </header>
  )
}

function Panel(props: React.ComponentProps<typeof MemoryChipPanel>) {
  return (
    <div className="flex w-[26rem] flex-col gap-3 rounded-xl bg-popover p-3 font-inter text-sm text-popover-foreground shadow-md ring-1 ring-popover-edge">
      <MemoryChipPanel {...props} />
    </div>
  )
}

const meta: Meta = {
  title: 'Chat/Memory cards',
  parameters: { layout: 'fullscreen' },
}
export default meta

export const Timeline: StoryObj = {
  render: () => (
    <Transcript>
      <ModelNoteMarker {...noteProps('recall', recall)} />
      <ModelNoteMarker {...noteProps('rules', rulesUpdate)} />
      <Header />
      <p className="chat-prose">Resposta do agente…</p>
    </Transcript>
  ),
}

export const ChipPanel: StoryObj = {
  render: () => (
    <Transcript>
      <div className="flex flex-wrap items-start gap-6">
        <Panel
          bank="segralim-d86496"
          semantic
          rules={2}
          load={{ state: 'done', memories: loaded }}
          retry={() => {}}
        />
        <div className="flex flex-col gap-6">
          <Panel
            bank="segralim-d86496"
            rules={0}
            load={{ state: 'loading' }}
            retry={() => {}}
          />
          <Panel
            bank="segralim-d86496"
            truncated
            rules={0}
            load={{ state: 'error' }}
            retry={() => {}}
          />
          <Panel
            bank="segralim-d86496"
            rules={0}
            load={{ state: 'done', memories: [] }}
            retry={() => {}}
          />
        </div>
      </div>
    </Transcript>
  ),
}
