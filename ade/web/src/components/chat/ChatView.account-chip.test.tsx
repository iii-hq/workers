// @vitest-environment jsdom
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatBackend } from '@/lib/backend'
import { seedClaudeAccountsForTests } from '@/lib/backend/claude-accounts'
import { usageFixture } from '@/stories/fixtures/claude-accounts-fixtures'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({
  /** The tab's loaded conversations (a sub-agent's parent is looked up here). */
  conversations: [] as unknown[],
  call: vi.fn<(id: string, payload: unknown) => Promise<unknown>>(),
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ call: mocks.call }),
}))
vi.mock('@/lib/conversations-context', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  useConversationsCtxOptional: () => ({
    conversations: mocks.conversations,
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
// The transcript reads which kind of locked conversation it is in.
vi.mock('./MessageList', async (importOriginal) => {
  const { useContext } = await import('react')
  const { LockedAccountContext } = await import('@/lib/limit-card')
  return {
    ...(await importOriginal<object>()),
    MessageList: () => (
      <div data-locked={useContext(LockedAccountContext) ?? 'none'} />
    ),
  }
})

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
  mocks.conversations = []
  mocks.call.mockReset().mockResolvedValue(null)
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

async function open(
  conversation: Conversation,
  props: Partial<ComponentProps<typeof ChatView>> = {},
) {
  const noop = vi.fn()
  await act(async () => {
    root.render(
      <TooltipProvider>
        <ChatView
          key={conversation.id}
          conversation={conversation}
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
          {...props}
        />
      </TooltipProvider>,
    )
  })
}

it("shows the chip and Details' account row for a conversation without a model", async () => {
  await open(sent)
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

it('tells the transcript a side chat from a sub-agent', async () => {
  const locked = () =>
    host.querySelector<HTMLElement>('[data-locked]')?.dataset.locked
  await open(sent)
  expect(locked()).toBe('none')
  await open({ ...sent, id: 'child', parentId: 'sent-elsewhere' })
  expect(locked()).toBe('sub-agent')
  await open({ ...sent, id: 'side' }, { side: { since: 1, empty: null } })
  expect(locked()).toBe('side-chat')
})

/* Locked conversations run where the engine runs them (the Android review):
   the fixture's global is Work; Personal is at its 5-hour limit. */

it('reads a side chat on the account its lock froze, not the global', async () => {
  await open(
    {
      ...sent,
      id: 'side',
      sessionMetadata: {
        side_chat_of: 'root',
        side_chat_lock: {
          model: 'claude-code::claude-opus-4-5',
          account: 'personal',
        },
      },
    },
    { side: { since: 1, empty: null } },
  )
  // The card names the lock's account: Personal's reading is at 100 %.
  const card = host.querySelector('[data-limit-card]')
  expect(card?.querySelector('h3')?.textContent).toBe(
    'Personal reached its 5-hour limit',
  )
  expect(card?.textContent).toContain(
    "This side chat stays on Personal so it keeps the conversation's cache.",
  )
  // No chip in a side chat.
  expect(host.querySelector('[data-account-chip]')).toBeNull()
})

it("reads a sub-agent on its parent's account", async () => {
  mocks.conversations = [
    {
      ...sent,
      id: 'parent',
      messages: [],
      sessionMetadata: { claude_account: 'personal' },
    },
  ]
  await open({ ...sent, id: 'child', parentId: 'parent' })
  const chip = host.querySelector<HTMLButtonElement>('[data-account-chip]')
  expect(chip?.textContent).toBe('Personal')
  expect(host.querySelector('[data-limit-card] h3')?.textContent).toBe(
    'Personal reached its 5-hour limit',
  )
  await act(async () => chip?.click())
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  expect(
    document.body.querySelector('[data-account-resolution]')?.textContent,
  ).toBe("Runs on Personal, its conversation's account.")
  // The parent is loaded: no status read.
  expect(mocks.call).not.toHaveBeenCalledWith(
    'harness::status',
    expect.anything(),
  )
})

it("reads a sub-agent whose parent isn't loaded from the parent turn's status", async () => {
  mocks.call.mockImplementation(async (id, payload) =>
    id === 'harness::status' &&
    (payload as { session_id?: string }).session_id === 'parent'
      ? { account: { id: 'personal', source: 'project' } }
      : null,
  )
  await open({ ...sent, id: 'child', parentId: 'parent' })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  expect(mocks.call).toHaveBeenCalledWith('harness::status', {
    session_id: 'parent',
    verbose: true,
  })
  expect(host.querySelector('[data-account-chip]')?.textContent).toBe(
    'Personal',
  )
})

it('falls back to its own account without the parent or its status', async () => {
  await open({ ...sent, id: 'child', parentId: 'parent' })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  expect(host.querySelector('[data-account-chip]')?.textContent).toBe('Work')
})
