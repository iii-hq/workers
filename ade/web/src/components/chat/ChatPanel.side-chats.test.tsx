// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { sideChatsMadeHere } from '@/lib/chat-aside'
import type { Conversation } from '@/types/chat'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  ctx: null as Record<string, unknown> | null,
}))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: () => mocks.ctx,
}))
vi.mock('./ChatView', () => ({
  ChatView: ({ conversation }: { conversation: Conversation }) => (
    <div data-chat-conversation-id={conversation.id} />
  ),
}))

import { ChatPanel } from './ChatPanel'

const DAY = 86_400_000

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

function makeCtx(conversations: Conversation[], activeId: string) {
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
    conversationsLoading: false,
    conversationsError: null,
    conversationLoadErrors: {},
    retryConversations: vi.fn(),
    missingConversationIds: new Set<string>(),
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
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

describe('ChatPanel side chats', () => {
  it('discards on leave only the side chats this tab made', () => {
    const list = [
      chat('a'),
      chat('b'),
      side('mine', 'a'),
      side('other-tab', 'a'),
    ]
    sideChatsMadeHere.add('mine')
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    expect(ctx.remove).not.toHaveBeenCalled()

    mocks.ctx = { ...ctx, activeId: 'b', active: list[1] }
    render()
    expect(ctx.remove).toHaveBeenCalledWith('mine')
    expect(ctx.remove).not.toHaveBeenCalledWith('other-tab')
  })

  it('keeps them while a pinned pane still shows the conversation', () => {
    const list = [chat('a'), chat('b'), side('mine', 'a')]
    sideChatsMadeHere.add('mine')
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render('a')
    mocks.ctx = { ...ctx, activeId: 'b', active: list[1] }
    render('a')
    expect(ctx.remove).not.toHaveBeenCalled()
  })

  it('never discards the side chat being shown', () => {
    const list = [chat('a'), side('mine', 'a')]
    sideChatsMadeHere.add('mine')
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    mocks.ctx = { ...ctx, activeId: 'mine', active: list[1] }
    render()
    expect(ctx.remove).not.toHaveBeenCalled()
  })

  it('sweeps side chats left untouched for a day, once', () => {
    const list = [
      chat('a'),
      side('stale', 'gone', { updatedAt: Date.now() - 2 * DAY }),
      side('fresh', 'gone'),
    ]
    const ctx = makeCtx(list, 'a')
    mocks.ctx = ctx
    render()
    render()
    expect(ctx.remove).toHaveBeenCalledTimes(1)
    expect(ctx.remove).toHaveBeenCalledWith('stale')
  })
})
