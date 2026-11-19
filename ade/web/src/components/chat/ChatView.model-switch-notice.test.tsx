// @vitest-environment jsdom

import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { realBackend } from '@/lib/backend/real'
import type { Conversation } from '@/types/chat'
import { ChatView } from './ChatView'

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: vi.fn() }),
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => null,
}))
vi.mock('./Composer', () => ({
  Composer: () => <textarea />,
}))
const modelSwitch = vi.hoisted(() => ({
  state: {
    request: () => Promise.resolve(true),
    dialog: null,
    pending: true,
    error: null as string | null,
    notice: null as string | null,
  },
}))
vi.mock('./use-model-switch', () => ({
  useModelSwitch: () => modelSwitch.state,
}))

const conversation: Conversation = {
  id: 'console-compose',
  title: 'Compose',
  model: 'openai-codex::codex/gpt-6-astra',
  hydrated: true,
  started: true,
  messages: [
    { id: 'e_reply', role: 'assistant', content: 'Done.', createdAt: 1 },
  ],
  createdAt: 1,
  updatedAt: 2,
}

function render() {
  return renderToStaticMarkup(
    <TooltipProvider>
      <ChatView
        conversation={conversation}
        backend={realBackend}
        modelOptions={[
          {
            id: 'openai-codex::codex/gpt-6-astra',
            label: 'GPT-6 Astra',
            contextWindow: 1_000_000,
          },
        ]}
        catalogLoading={false}
        onUpdateModel={vi.fn()}
        onUpdateThinkingLevel={vi.fn()}
        onUpdateWorkingDir={vi.fn()}
        onAppendMessage={vi.fn()}
        onPatchMessage={vi.fn()}
        onCompactConversation={vi.fn()}
      />
    </TooltipProvider>,
  )
}

/**
 * Where the status panel holding `text` sits: parsed, so a panel right after
 * the composer column's closing tag reads as outside it.
 */
function placement(html: string, text: string) {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  const column = doc.querySelector('footer > div[class~="max-w-[760px]"]')
  const panel = [
    ...doc.querySelectorAll('[role="status"], [role="alert"]'),
  ].find((element) => element.textContent?.includes(text))
  return {
    found: panel !== undefined,
    inColumn: column !== null && panel !== undefined && column.contains(panel),
  }
}

it('reads a panel after the composer column as outside it', () => {
  const panel = '<div role="status">Switching models</div>'
  const column = '<div class="mx-auto max-w-[760px]"><textarea></textarea>'

  expect(
    placement(`<footer>${column}${panel}</div></footer>`, 'Switching models'),
  ).toEqual({ found: true, inColumn: true })
  expect(
    placement(`<footer>${column}</div>${panel}</footer>`, 'Switching models'),
  ).toEqual({ found: true, inColumn: false })
})

beforeEach(() => {
  modelSwitch.state.pending = true
  modelSwitch.state.error = null
  modelSwitch.state.notice = null
})

it('shows the model-switch progress inside the composer column', () => {
  modelSwitch.state.notice = 'Checking the selected model’s context…'

  const html = render()

  expect(placement(html, 'Switching models')).toEqual({
    found: true,
    inColumn: true,
  })
})

it('shows a failed model switch inside the composer column', () => {
  modelSwitch.state.error = 'The conversation is too large for this model.'

  const html = render()

  expect(placement(html, 'Model switch not completed')).toEqual({
    found: true,
    inColumn: true,
  })
})

it('shows nothing while no model switch is in flight', () => {
  modelSwitch.state.pending = false

  const html = render()

  expect(html).not.toContain('Switching models')
  expect(html).not.toContain('Model switch not completed')
})
