// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({ useConversationsCtx: vi.fn() }))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: mocks.useConversationsCtx,
}))
// A desktop pane under the narrow threshold; not a phone.
vi.mock('@/hooks/use-container-narrow', () => ({
  useContainerNarrow: () => [() => {}, true],
}))
vi.mock('@/hooks/use-media-query', () => ({ useMediaQuery: () => false }))
vi.mock('@/components/sidebar/ConversationSidebar', () => ({
  ConversationSidebar: () => <div data-sidebar />,
}))
vi.mock('./ChatView', () => ({
  ChatView: ({
    conversation,
    onBack,
  }: {
    conversation: Conversation
    onBack?: () => void
  }) => (
    <button
      type="button"
      data-chat-conversation-id={conversation.id}
      onClick={onBack}
    />
  ),
}))

import { TooltipProvider } from '@/components/ui/Tooltip'
import { ChatPanel } from './ChatPanel'

// A fresh element each time, so a re-render reads the mocked context again.
const panel = () => (
  <TooltipProvider>
    <ChatPanel density="dock" />
  </TooltipProvider>
)

function conversation(id: string): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 1,
    updatedAt: 1,
  }
}

function ctx(ids: string[], activeId: string) {
  const conversations = ids.map(conversation)
  return {
    conversations,
    activeId,
    active: conversations.find((c) => c.id === activeId) ?? null,
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
  }
}

let root: Root | null = null
let host: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  host = document.body.appendChild(document.createElement('div'))
  root = createRoot(host)
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  host.remove()
})

/** Open `activeId`, then go back to the session list. */
async function onTheList(ids: string[], activeId: string) {
  mocks.useConversationsCtx.mockReturnValue(ctx(ids, activeId))
  await act(async () => root?.render(panel()))
  await act(async () =>
    host
      .querySelector<HTMLButtonElement>('[data-chat-conversation-id]')
      ?.click(),
  )
  expect(host.querySelector('[data-chat-conversation-id]')).toBeNull()
}

async function rerender(ids: string[], activeId: string) {
  mocks.useConversationsCtx.mockReturnValue(ctx(ids, activeId))
  await act(async () => root?.render(panel()))
}

describe('ChatPanel in a narrow desktop pane', () => {
  it('follows a chat opened from outside (a setup wizard example) out of the list', async () => {
    await onTheList(['older', 'current'], 'current')
    await rerender(['example', 'older', 'current'], 'example')
    expect(
      host.querySelector('[data-chat-conversation-id="example"]'),
    ).not.toBeNull()
  })

  it('stays on the list when the open chat is deleted from it', async () => {
    await onTheList(['older', 'current'], 'current')
    await rerender(['older'], 'older')
    expect(host.querySelector('[data-chat-conversation-id]')).toBeNull()
  })
})
