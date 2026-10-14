// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { sideChatsMadeHere } from '@/lib/chat-aside'
import type { Conversation } from '@/types/chat'
import type { AsideRequest } from './ChatAside'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  ctx: null as Record<string, unknown> | null,
  trigger: vi.fn(),
  fetchTranscriptTail: vi.fn(),
  setSessionMeta: vi.fn(),
  deleteSession: vi.fn(),
}))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: () => mocks.ctx,
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ trigger: mocks.trigger }),
}))
vi.mock('@/lib/sessions/api', () => ({
  fetchTranscriptTail: mocks.fetchTranscriptTail,
  setSessionMeta: mocks.setSessionMeta,
  deleteSession: mocks.deleteSession,
}))
vi.mock('./ChatView', () => ({
  ChatView: ({
    conversation,
    side,
  }: {
    conversation: Conversation
    side?: { autoFocus?: boolean }
  }) => (
    <div
      data-side-chat={conversation.id}
      data-autofocus={String(side?.autoFocus ?? false)}
    />
  ),
}))
vi.mock('./ChatContextPanel', () => ({
  ChatContextPanel: ({ active }: { active?: boolean }) => (
    <div data-context-panel data-active={String(active)} />
  ),
}))

import { ChatAside } from './ChatAside'

function chat(id: string, extra: Partial<Conversation> = {}): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: 10,
    updatedAt: 10,
    ...extra,
  }
}

const main = chat('main', {
  title: 'Main',
  started: true,
  messages: [{ id: 'e1', role: 'user', content: 'hi', createdAt: 1 }],
})
const fork = chat('fork-1', {
  createdAt: 20,
  sessionMetadata: { side_chat_of: 'main' },
})

function makeCtx(conversations: Conversation[]) {
  return {
    conversations,
    connectionState: 'connected',
    watchConversation: vi.fn(() => () => {}),
    setDraftText: vi.fn(),
    getDraftText: vi.fn(() => undefined as string | undefined),
    setSessionMetadata: vi.fn(),
    remove: vi.fn(() => Promise.resolve()),
    openConversationInPanel: vi.fn(),
    backend: { id: 'real' },
    modelOptions: [],
    catalogLoading: false,
    setModel: vi.fn(),
    setThinkingLevel: vi.fn(),
    setWorkingDir: vi.fn(),
    appendMessage: vi.fn(),
    updateMessage: vi.fn(),
    compactConversation: vi.fn(),
    shellAvailable: true,
    githubAvailable: false,
  }
}

let root: Root | null = null
let container: HTMLElement | null = null

function render(request: AsideRequest | null = null) {
  if (!container) {
    container = document.createElement('div')
    document.body.appendChild(container)
    root = createRoot(container)
  }
  act(() =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ChatAside
          conversation={main}
          side="right"
          phone={false}
          sheetOpen={false}
          onSheetOpenChange={() => {}}
          request={request}
          triggers={[]}
          onUnregisterTrigger={() => {}}
        />
      </TooltipProvider>,
    ),
  )
  return container
}

async function settle() {
  for (let i = 0; i < 4; i += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
}

const button = (label: string) =>
  [...document.querySelectorAll('button')].find(
    (el) => el.getAttribute('aria-label') === label || el.textContent === label,
  ) as HTMLButtonElement | undefined

beforeEach(() => {
  window.localStorage.clear()
  sideChatsMadeHere.clear()
  mocks.trigger.mockReset()
  mocks.fetchTranscriptTail.mockReset()
  mocks.setSessionMeta.mockReset()
  mocks.deleteSession.mockReset()
  mocks.fetchTranscriptTail.mockResolvedValue({
    items: [{ entry_id: 'e1' }],
    hasMore: false,
  })
  mocks.setSessionMeta.mockResolvedValue({})
  mocks.deleteSession.mockResolvedValue({ deleted: true })
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

describe('ChatAside side chat', () => {
  it('forks once, tags the fork and drops its links to a parent', async () => {
    let resolveFork: (value: unknown) => void = () => {}
    mocks.trigger.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveFork = resolve
        }),
    )
    mocks.ctx = makeCtx([main])
    render({ seq: 1, tab: 'side-chat' })
    render({ seq: 2, tab: 'side-chat' })
    await settle()
    resolveFork({
      session_id: 'fork-1',
      meta: {
        created_at: 20,
        metadata: {
          agent_profile: { id: 'p' },
          parent_session_id: 'boss',
          depth: 1,
        },
      },
    })
    await settle()

    expect(mocks.trigger).toHaveBeenCalledTimes(1)
    expect(mocks.trigger).toHaveBeenCalledWith('session::fork', {
      session_id: 'main',
      entry_id: 'e1',
      title: 'Side chat · Main',
    })
    expect(mocks.setSessionMeta).toHaveBeenCalledWith({
      session_id: 'fork-1',
      metadata: { agent_profile: { id: 'p' }, side_chat_of: 'main' },
    })
    expect(sideChatsMadeHere.has('fork-1')).toBe(true)
    // The fork is not in the list yet: no second "Start" to click meanwhile.
    expect(button('Start side chat')).toBeUndefined()

    mocks.ctx = makeCtx([main, fork])
    const view = render({ seq: 2, tab: 'side-chat' })
    expect(view.querySelector('[data-side-chat="fork-1"]')).not.toBeNull()
  })

  it('deletes a fork whose tag could not be written, and says so', async () => {
    mocks.trigger.mockResolvedValue({
      session_id: 'fork-1',
      meta: { created_at: 20, metadata: {} },
    })
    mocks.setSessionMeta.mockRejectedValue(new Error('set-meta down'))
    mocks.ctx = makeCtx([main])
    const view = render({ seq: 1, tab: 'side-chat' })
    await settle()
    expect(mocks.deleteSession).toHaveBeenCalledWith('fork-1')
    expect(view.textContent).toContain('Side chat not started')
  })

  it('puts a quote before what was already typed and focuses the composer', async () => {
    const ctx = makeCtx([main, fork])
    ctx.getDraftText = vi.fn(() => 'my words')
    mocks.ctx = ctx
    const view = render({ seq: 1, tab: 'side-chat', quote: '> q\n\n' })
    await settle()
    expect(ctx.setDraftText).toHaveBeenCalledWith('fork-1', '> q\n\nmy words')
    expect(
      view
        .querySelector('[data-side-chat="fork-1"]')
        ?.getAttribute('data-autofocus'),
    ).toBe('true')
  })

  it('does not take the caret when it just shows a side chat', () => {
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    mocks.ctx = makeCtx([main, fork])
    const view = render()
    expect(
      view
        .querySelector('[data-side-chat="fork-1"]')
        ?.getAttribute('data-autofocus'),
    ).toBe('false')
  })

  it('hides a discarded side chat at once, before the delete lands', async () => {
    const ctx = makeCtx([main, fork])
    ctx.remove = vi.fn(() => new Promise<void>(() => {}))
    mocks.ctx = ctx
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    const view = render()
    act(() => button('Discard side chat')?.click())
    expect(ctx.remove).toHaveBeenCalledWith('fork-1')
    expect(view.querySelector('[data-side-chat]')).toBeNull()
  })

  it('keeps a side chat by removing its tag', () => {
    const ctx = makeCtx([main, fork])
    mocks.ctx = ctx
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    const view = render()
    act(() => button('Keep as conversation')?.click())
    expect(ctx.setSessionMetadata).toHaveBeenCalledWith('fork-1', {
      side_chat_of: undefined,
    })
    expect(view.textContent).toContain('Kept as a conversation')
  })
})

describe('ChatAside context', () => {
  it('lets the context read only while the panel is open', async () => {
    mocks.ctx = makeCtx([main])
    const view = render()
    await settle()
    const panel = () => view.querySelector('[data-context-panel]')
    expect(panel()?.getAttribute('data-active')).toBe('true')
    act(() => button('collapse Side panel')?.click())
    await settle()
    expect(panel()?.getAttribute('data-active')).not.toBe('true')
  })
})
