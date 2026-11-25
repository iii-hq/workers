// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import { usageFixture } from '@/stories/fixtures/claude-accounts-fixtures'
import type { Conversation } from '@/types/chat'

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: async () => null }),
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => ({
    conversations: [],
    connectionState: 'connected',
    harnessStatus: {
      present: true,
      loading: false,
      installing: false,
      error: null,
    },
    getDraftText: () => undefined,
    getDraftAttachments: () => undefined,
    setDraftText: vi.fn(),
    setDraftAttachments: vi.fn(),
    saveDetails: vi.fn(),
    shellAvailable: false,
    worktreeAvailable: false,
    memoryAvailable: false,
    approvalGateAvailable: false,
  }),
}))
vi.mock('./Composer', () => ({ Composer: () => <textarea /> }))
vi.mock('./ChatAside', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  ChatAside: () => null,
}))
vi.mock('./MessageList', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  MessageList: () => <div />,
}))

import { ChatView } from './ChatView'

const NOW = Date.now()

/*
 * Turns another client sent through harness::send: no `metadata.model`, the
 * transcript's reply names its provider. The chip shows, and opens Details
 * on the account row (the live acceptance found the row missing).
 */
const sent: Conversation = {
  id: 'sent-elsewhere',
  title: 'Fix the parser',
  model: null,
  status: 'done',
  hydrated: true,
  started: true,
  messages: [
    { id: 'e_idem_m1', role: 'user', content: 'Go', createdAt: NOW - 2_000 },
    {
      id: 'e_t_1_0_assistant:0',
      role: 'assistant',
      content: 'Done',
      model: 'claude-opus-4-5',
      provider: 'claude-code',
      createdAt: NOW - 1_000,
    },
  ],
  createdAt: 1,
  updatedAt: 2,
}

let root: Root
let host: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('min-width'),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
  seedClaudeAccountsForTests({ usage: usageFixture(NOW) })
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  document.body.innerHTML = ''
  seedClaudeAccountsForTests(null)
  vi.unstubAllGlobals()
})

it("shows the chip and Details' account row for a conversation without a model", async () => {
  const noop = vi.fn()
  await act(async () => {
    root.render(
      <TooltipProvider>
        <ChatView
          conversation={sent}
          backend={{ id: 'real', stream: vi.fn() } as unknown as ChatBackend}
          modelOptions={[
            { id: 'anthropic::claude-opus-4-5', label: 'Claude Opus 4.5' },
            { id: 'claude-code::claude-opus-4-5', label: 'Claude Opus 4.5' },
          ]}
          catalogLoading={false}
          onUpdateModel={noop}
          onUpdateThinkingLevel={noop}
          onUpdateWorkingDir={noop}
          onAppendMessage={noop}
          onPatchMessage={noop}
          onCompactConversation={noop}
        />
      </TooltipProvider>,
    )
  })
  const chip = host.querySelector<HTMLButtonElement>('[data-account-chip]')
  expect(chip?.textContent).toBe('Work')
  await act(async () => chip?.click())
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  const row = document.body.querySelector('[data-conversation-account]')
  expect(row).not.toBeNull()
  expect(
    document.body.querySelector('[data-account-resolution]')?.textContent,
  ).toContain('Runs on Work')
})
