// @vitest-environment jsdom
import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { seedChatSettingsForTests } from '@/hooks/use-chat-settings'
import type { SnapshotAutocompact } from '@/lib/autocompact'
import type { ChatBackend } from '@/lib/backend'
import type { ContextUsageReport } from '@/lib/backend/types'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
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

/* chat-projects §3.3.3: "Auto-compact 160k" next to the account chip, on
   wide surfaces, only when a size below the model's limit applies; a tap
   opens Details on the Auto-compact row. */
const chat: Conversation = {
  id: 'c-auto',
  title: 'Fix the parser',
  model: 'anthropic::claude-sonnet-5-5',
  status: 'done',
  hydrated: true,
  started: true,
  messages: [
    { id: 'e_idem_m1', role: 'user', content: 'Go', createdAt: NOW - 2_000 },
  ],
  createdAt: 1,
  updatedAt: 2,
}

let root: Root
let host: HTMLDivElement
let wide = true

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  wide = true
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: wide && query.includes('min-width'),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }))
  setChatCapabilitiesForTests({ autocompact: true, chatSettings: true })
  seedChatSettingsForTests({
    descriptions: { agent_writes: true },
    autocompact: { mode: 'tokens', tokens: 400_000 },
  })
  mocks.conversations = []
  turnObservers = []
  backend = null
  // No engine: the seeded settings stand.
  mocks.call.mockReset().mockRejectedValue(new Error('no engine in tests'))
  host = document.createElement('div')
  document.body.append(host)
  root = createRoot(host)
})

afterEach(async () => {
  await act(async () => root.unmount())
  host.remove()
  document.body.innerHTML = ''
  setChatCapabilitiesForTests(null)
  seedChatSettingsForTests(null)
  vi.unstubAllGlobals()
})

/** The turn snapshot observers ChatView opened (`onTurnAutocompact`). */
let turnObservers: {
  sessionId: string
  onValue: (value: SnapshotAutocompact) => void
  closed: boolean
}[] = []
const onTurnAutocompact = (
  sessionId: string,
  onValue: (value: SnapshotAutocompact) => void,
) => {
  const observer = { sessionId, onValue, closed: false }
  turnObservers.push(observer)
  return () => {
    observer.closed = true
  }
}
const backendFor = (report: ContextUsageReport) =>
  ({
    id: 'real',
    stream: vi.fn(),
    contextUsage: async () => report,
    onTurnAutocompact,
  }) as unknown as ChatBackend
let backend: ChatBackend | null = null

async function open(
  conversation: Conversation,
  report: ContextUsageReport,
  props: Partial<ComponentProps<typeof ChatView>> = {},
) {
  const noop = vi.fn()
  backend ??= backendFor(report)
  const shared = backend
  await act(async () => {
    root.render(
      <TooltipProvider>
        <ChatView
          key={conversation.id}
          conversation={conversation}
          backend={shared}
          modelOptions={[
            {
              id: 'anthropic::claude-sonnet-5-5',
              label: 'Claude Sonnet 5.5',
            },
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

const chip = () =>
  host.querySelector<HTMLButtonElement>('[data-autocompact-chip]')
const ONE_MILLION = { total: 142_000, usable: 916_000, free: 774_000 }

it('shows the chip and opens Details on the Auto-compact row', async () => {
  await open(chat, ONE_MILLION)
  expect(chip()?.textContent).toBe('Auto-compact 400k')
  await act(async () => chip()?.click())
  const row = () =>
    document.body.querySelector('[data-conversation-autocompact]')
  // Details focus the row once the dialog is open.
  await act(async () => {
    for (let i = 0; i < 100 && !row()?.contains(document.activeElement); i++)
      await new Promise((resolve) => requestAnimationFrame(resolve))
  })
  expect(row()).not.toBeNull()
  expect(row()?.contains(document.activeElement)).toBe(true)
})

it("reads the conversation's own size first", async () => {
  await open(
    {
      ...chat,
      sessionMetadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
    },
    ONE_MILLION,
  )
  expect(chip()?.textContent).toBe('Auto-compact 600k')
})

it("has no chip at the model's limit: 400k on a 200k model", async () => {
  await open(chat, { total: 90_000, usable: 116_000, free: 26_000 })
  expect(chip()).toBeNull()
})

it('has no chip when it is off', async () => {
  seedChatSettingsForTests({
    descriptions: { agent_writes: true },
    autocompact: { mode: 'off', tokens: 400_000 },
  })
  await open(chat, ONE_MILLION)
  expect(chip()).toBeNull()
})

it('has no chip on a narrow surface (D138, D169)', async () => {
  wide = false
  await open(chat, ONE_MILLION)
  expect(chip()).toBeNull()
})

it('has no chip in a side chat', async () => {
  await open(chat, ONE_MILLION, { side: { since: 1, empty: null } })
  expect(chip()).toBeNull()
})

it('has no chip on a harness without auto-compact (D168)', async () => {
  setChatCapabilitiesForTests({ autocompact: false, chatSettings: true })
  await open(chat, ONE_MILLION)
  expect(chip()).toBeNull()
})

/* While a turn runs, the tick, the label and the chip read the value the
   turn froze, as the harness and the injected chip do; between turns, the
   shared resolution (§3.3.3, D164). */
it("reads the running turn's snapshot, and the resolution once it ends", async () => {
  await open(chat, ONE_MILLION)
  expect(chip()?.textContent).toBe('Auto-compact 400k')
  expect(turnObservers).toEqual([])
  // A turn starts: it keeps what it started with until a step writes.
  await open({ ...chat, status: 'working' }, ONE_MILLION)
  expect(chip()?.textContent).toBe('Auto-compact 400k')
  expect(turnObservers.map((o) => o.sessionId)).toEqual(['c-auto'])
  await act(async () =>
    turnObservers[0]?.onValue({
      setting_tokens: 600_000,
      effective_tokens: 600_000,
      cap_tokens: 660_000,
      rule: { scope: 'session' },
    }),
  )
  expect(chip()?.textContent).toBe('Auto-compact 600k')
  expect(host.querySelector('[data-autocompact-label]')?.textContent).toContain(
    'auto 600k',
  )
  // A change made during the turn waits for its end.
  await act(async () =>
    seedChatSettingsForTests({
      descriptions: { agent_writes: true },
      autocompact: { mode: 'tokens', tokens: 200_000 },
    }),
  )
  expect(chip()?.textContent).toBe('Auto-compact 600k')
  // The turn ends: the shared resolution, with the change.
  await open({ ...chat, status: 'done' }, ONE_MILLION)
  expect(turnObservers[0]?.closed).toBe(true)
  expect(chip()?.textContent).toBe('Auto-compact 200k')
})

// Opened as a chat of its own (not beside its conversation), a side chat
// has the header: the chip and the meter follow its conversation.
describe("a side chat whose conversation isn't loaded here", () => {
  const side: Conversation = {
    ...chat,
    id: 'c-side',
    // The copy frozen at the fork: never what the meter shows.
    sessionMetadata: {
      side_chat_of: 'c-root',
      autocompact: { mode: 'tokens', tokens: 100_000 },
    },
  }
  const label = () =>
    host.querySelector('[data-autocompact-label]')?.textContent ?? null

  it('reads it from the engine', async () => {
    mocks.call.mockImplementation(async (fn: string, payload: unknown) => {
      if (
        fn === 'session::get' &&
        (payload as { session_id?: string }).session_id === 'c-root'
      ) {
        return {
          meta: {
            session_id: 'c-root',
            metadata: { autocompact: { mode: 'tokens', tokens: 600_000 } },
          },
        }
      }
      throw new Error('no engine in tests')
    })
    await open(side, ONE_MILLION)
    await act(async () => {
      for (let i = 0; i < 20 && label() === null; i++)
        await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(label()).toContain('auto 600k')
    expect(chip()?.textContent).toBe('Auto-compact 600k')
  })

  it("shows no value when it can't be read", async () => {
    await open(side, ONE_MILLION)
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
    expect(mocks.call).toHaveBeenCalledWith(
      'session::get',
      { session_id: 'c-root' },
      expect.anything(),
    )
    expect(label()).toBeNull()
    expect(chip()).toBeNull()
    expect(host.querySelector('[data-ring-tick]')).toBeNull()
  })
})
