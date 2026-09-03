import { renderToStaticMarkup } from 'react-dom/server'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({
  useConversationsCtx: vi.fn(),
}))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: mocks.useConversationsCtx,
}))

vi.mock('./ChatView', () => ({
  ChatView: ({
    conversation,
    panelTitle,
  }: {
    conversation: Conversation
    panelTitle?: string
  }) => (
    <div
      data-chat-conversation-id={conversation.id}
      data-panel-title={panelTitle}
    />
  ),
}))

import { ChatPanel } from './ChatPanel'

function conversation(id: string, title: string): Conversation {
  return {
    id,
    title,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 1,
    updatedAt: 1,
  }
}

describe('ChatPanel conversationId', () => {
  const active = conversation('active-session', 'Active chat')
  const child = conversation('child-session', 'Frontend')

  beforeEach(() => {
    mocks.useConversationsCtx.mockReturnValue({
      conversations: [active, child],
      activeId: active.id,
      active,
      watchConversation: vi.fn(() => vi.fn()),
      createNew: vi.fn(),
      select: vi.fn(),
      rename: vi.fn(),
      remove: vi.fn(),
      setModel: vi.fn(),
      setWorkingDir: vi.fn(),
      appendMessage: vi.fn(),
      updateMessage: vi.fn(),
      compactConversation: vi.fn(),
      backend: {},
      modelOptions: [],
      catalogLoading: false,
      connectionState: 'connected',
      missingConversationIds: new Set<string>(),
    })
  })

  it('renders the pinned session without replacing the globally active chat', () => {
    const html = renderToStaticMarkup(
      <ChatPanel conversationId={child.id} density="dock" />,
    )

    expect(html).toContain('data-chat-conversation-id="child-session"')
    expect(html).toContain('data-panel-title="Frontend"')
    expect(html).not.toContain('data-chat-conversation-id="active-session"')
  })

  it('does not silently fall back to the active chat while a deep link resolves', () => {
    const html = renderToStaticMarkup(
      <ChatPanel conversationId="not-loaded-yet" density="dock" />,
    )

    expect(html).toContain('Loading conversation…')
    expect(html).not.toContain('data-chat-conversation-id')
  })

  it('distinguishes a confirmed missing deep link from one still loading', () => {
    mocks.useConversationsCtx.mockReturnValue({
      ...mocks.useConversationsCtx(),
      missingConversationIds: new Set(['deleted-session']),
    })

    const html = renderToStaticMarkup(
      <ChatPanel conversationId="deleted-session" density="dock" />,
    )

    expect(html).toContain('Conversation not found.')
    expect(html).not.toContain('Loading conversation…')
  })
})
