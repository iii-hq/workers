// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { sideChatsMadeHere } from '@/lib/chat-aside'
import { setChatCapabilitiesForTests } from '@/lib/chat-capabilities'
import type { Conversation } from '@/types/chat'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  ctx: null as Record<string, unknown> | null,
  leftByClosedTabs: [] as string[],
  closeSideChat: vi.fn(),
}))

vi.mock('@/lib/backend/side-chats', () => ({
  closeSideChat: mocks.closeSideChat,
}))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: () => mocks.ctx,
}))
vi.mock('@/lib/chat-aside', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  sideChatsLeftByClosedTabs: () => Promise.resolve(mocks.leftByClosedTabs),
}))
vi.mock('./ChatView', () => ({
  ChatView: ({ conversation }: { conversation: Conversation }) => (
    <div data-chat-conversation-id={conversation.id} />
  ),
}))

import { ChatPanel } from './ChatPanel'

const HOUR = 3_600_000

function chat(id: string, extra: Partial<Conversation> = {}): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: Date.now(),
    updatedAt: Date.now(),
    ...extra,
  }
}

const side = (id: string, of: string, extra: Partial<Conversation> = {}) =>
  chat(id, { sessionMetadata: { side_chat_of: of }, ...extra })

function makeCtx(
  conversations: Conversation[],
  activeId: string,
  missing: string[] = [],
  loading = false,
) {
  return {
    conversations,
    activeId,
    active: conversations.find((c) => c.id === activeId) ?? null,
    watchConversation: () => () => {},
    createNew: vi.fn(),
    select: vi.fn(),
    rename: vi.fn(),
    remove: vi.fn(() => Promise.resolve()),
    setModel: vi.fn(),
    setThinkingLevel: vi.fn(),
    setWorkingDir: vi.fn(),
    appendMessage: vi.fn(),
    updateMessage: vi.fn(),
    compactConversation: vi.fn(),
    backend: { id: 'real' },
    modelOptions: [],
    catalogLoading: false,
    connectionState: 'connected',
    conversationsLoading: loading,
    conversationsError: null,
    conversationLoadErrors: {},
    retryConversations: vi.fn(),
    missingConversationIds: new Set(missing),
  }
}

let root: Root | null = null
let container: HTMLElement | null = null

function render(pinned?: string) {
  if (!container) {
    container = document.createElement('div')
    document.body.appendChild(container)
    root = createRoot(container)
  }
  act(() =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ChatPanel />
        {pinned && <ChatPanel conversationId={pinned} />}
      </TooltipProvider>,
    ),
  )
}

beforeEach(() => {
  sideChatsMadeHere.clear()
  mocks.leftByClosedTabs = []
  mocks.closeSideChat.mockReset()
  mocks.closeSideChat.mockResolvedValue({ deleted: false })
  // The catalogue has been read: an engine without side chats.
  setChatCapabilitiesForTests({})
})

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

afterEach(() => {
  setChatCapabilitiesForTests(null)
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

describe('ChatPanel side chats', () => {
  it('leaves decision copies to their question lifecycle even when old or the original is gone', async () => {
    const decision = side('decision', 'gone', {
      updatedAt: Date.now() - 3 * HOUR,
      sessionMetadata: { side_chat_of: 'gone', side_chat_question: 'gone/q' },
    })
    const ctx = makeCtx([chat('main'), decision], 'main', ['gone'])
    mocks.ctx = ctx
    render()
    await settle()
    expect(ctx.remove).not.toHaveBeenCalled()
  })
  it('keeps them when the conversation is left', async () => {
    const list = [chat('a'), chat('b'), side('mine', 'a')]
    sideChatsMadeHere.add('mine')
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    mocks.ctx = { ...ctx, activeId: 'b', active: list[1] }
    render()
    await settle()
    expect(ctx.remove).not.toHaveBeenCalled()
  })

  it('ends the ones untouched for two hours, unless a pane shows their chat', async () => {
    const idle = { updatedAt: Date.now() - 3 * HOUR }
    const list = [
      chat('a'),
      chat('b'),
      side('idle', 'b', idle),
      side('shown', 'a', idle),
      side('recent', 'b', { updatedAt: Date.now() - HOUR }),
    ]
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    render()
    await settle()
    expect(ctx.remove).toHaveBeenCalledTimes(1)
    expect(ctx.remove).toHaveBeenCalledWith('idle')
  })

  it('checks again while the ADE stays open', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    try {
      const list = [chat('a'), side('later', 'b')]
      const ctx = makeCtx(list, 'a')
      mocks.ctx = ctx
      render()
      await settle()
      expect(ctx.remove).not.toHaveBeenCalled()
      list[1].updatedAt = Date.now() - 3 * HOUR
      act(() => vi.advanceTimersByTime(5 * 60 * 1000))
      await settle()
      expect(ctx.remove).toHaveBeenCalledWith('later')
    } finally {
      vi.useRealTimers()
    }
  })

  it('never sweeps one that is still working', async () => {
    const list = [
      chat('a'),
      side('running', 'b', { updatedAt: 0, status: 'working' }),
    ]
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    await settle()
    expect(ctx.remove).not.toHaveBeenCalled()
  })

  it('ends them with their main chat', async () => {
    const list = [chat('a'), side('orphan', 'gone'), side('kept', 'a')]
    const ctx = makeCtx(list, 'a', ['gone'])
    mocks.ctx = ctx
    render()
    await settle()
    expect(ctx.remove).toHaveBeenCalledTimes(1)
    expect(ctx.remove).toHaveBeenCalledWith('orphan')
  })

  it('ends the ones left when every ADE tab closed, once the list is in', async () => {
    mocks.leftByClosedTabs = ['left', 'kept-elsewhere', 'unknown']
    for (const id of mocks.leftByClosedTabs) sideChatsMadeHere.add(id)
    const draft = [chat('draft')]
    const ctx = makeCtx(draft, 'draft', [], true)
    mocks.ctx = ctx
    render()
    render('a')
    await settle()
    expect(ctx.remove).not.toHaveBeenCalled()
    const list = [chat('a'), side('left', 'a'), chat('kept-elsewhere')]
    mocks.ctx = { ...makeCtx(list, 'a'), remove: ctx.remove }
    render('a')
    await settle()
    expect(ctx.remove).toHaveBeenCalledTimes(1)
    expect(ctx.remove).toHaveBeenCalledWith('left')
    expect(sideChatsMadeHere.has('kept-elsewhere')).toBe(false)
    expect(sideChatsMadeHere.has('unknown')).toBe(true)
  })

  it('sweeps nothing until the engine catalogue is read', async () => {
    setChatCapabilitiesForTests({ loaded: false })
    const list = [chat('a'), side('stale', 'b', { updatedAt: 0 })]
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    await settle()
    expect(ctx.remove).not.toHaveBeenCalled()
    expect(mocks.closeSideChat).not.toHaveBeenCalled()
  })

  describe('on an engine with side chats', () => {
    beforeEach(() => setChatCapabilitiesForTests({ sideChats: true }))

    it('no longer deletes one with messages after two hours: close keeps it', async () => {
      const idle = { updatedAt: Date.now() - 3 * HOUR }
      const list = [chat('a'), chat('b'), side('talked', 'b', idle)]
      const ctx = makeCtx(list, 'a')
      mocks.ctx = ctx
      render()
      await settle()
      // Asked once, through `close`, which deletes only an empty one.
      expect(mocks.closeSideChat).toHaveBeenCalledTimes(1)
      expect(mocks.closeSideChat).toHaveBeenCalledWith('talked')
      expect(ctx.remove).not.toHaveBeenCalled()
    })

    it('asks close, once, for the ones left when every tab closed', async () => {
      mocks.leftByClosedTabs = ['left-here']
      sideChatsMadeHere.add('left-here')
      const list = [chat('a'), side('left-here', 'a')]
      const ctx = makeCtx(list, 'a')
      mocks.ctx = ctx
      render('a')
      await settle()
      expect(mocks.closeSideChat).toHaveBeenCalledWith('left-here')
      expect(ctx.remove).not.toHaveBeenCalled()
    })

    it('leaves the side chats of a deleted conversation to the engine', async () => {
      const list = [chat('a'), side('orphan', 'gone')]
      const ctx = makeCtx(list, 'a', ['gone'])
      mocks.ctx = ctx
      render()
      await settle()
      expect(ctx.remove).not.toHaveBeenCalled()
    })
  })

  it('sweeps nothing until the server list is in', async () => {
    const list = [chat('a'), side('stale', 'b', { updatedAt: 0 })]
    const ctx = makeCtx(list, 'a', [], true)
    mocks.ctx = ctx
    render()
    await settle()
    expect(ctx.remove).not.toHaveBeenCalled()
    mocks.ctx = { ...ctx, conversationsLoading: false }
    render()
    await settle()
    expect(ctx.remove).toHaveBeenCalledWith('stale')
  })
})
