// @vitest-environment jsdom
import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Conversation } from '@/types/chat'

const mocks = vi.hoisted(() => ({ useConversationsCtx: vi.fn() }))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: mocks.useConversationsCtx,
}))
vi.mock('@/hooks/use-container-narrow', () => ({
  useContainerNarrow: () => [() => {}, false],
}))
vi.mock('@/hooks/use-media-query', () => ({
  DESKTOP_POINTER_QUERY: '(hover: hover) and (pointer: fine)',
  useMediaQuery: () => false,
}))
// The sidebar as one button per chat, so a test can pick one in a pane.
vi.mock('@/components/sidebar/ConversationSidebar', () => ({
  ConversationSidebar: ({
    conversations,
    onSelect,
  }: {
    conversations: Conversation[]
    onSelect: (id: string) => void
  }) => (
    <div>
      {conversations.map((c) => (
        <button
          key={c.id}
          type="button"
          data-pick={c.id}
          onClick={() => onSelect(c.id)}
        />
      ))}
    </div>
  ),
}))
vi.mock('./ChatView', () => ({
  ChatView: ({ conversation }: { conversation: Conversation }) => (
    <div data-chat-conversation-id={conversation.id} />
  ),
}))

import { TooltipProvider } from '@/components/ui/Tooltip'
import {
  chatPaneKey,
  resetChatPanesForTest,
  savePaneChat,
} from '@/lib/chat-pane-selection'
import { requestChatShown } from '@/lib/chat-requests'
import { ChatPanel } from './ChatPanel'

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

const conversations = ['chat-a', 'chat-b', 'chat-c'].map(conversation)
let globalActive: string | null = null

/**
 * The store's one active chat, live: `select` moves it and every pane
 * re-renders, the way the conversations provider does.
 */
function Store({
  initial,
  missing = [],
  children,
}: {
  initial: string
  /** Chats the store has looked up and found gone. */
  missing?: string[]
  /** Called on every store render: the mocked hook is no subscription. */
  children: () => React.ReactNode
}) {
  const [activeId, setActiveId] = useState<string | null>(initial)
  globalActive = activeId
  mocks.useConversationsCtx.mockImplementation(() => ({
    conversations,
    activeId,
    active: conversations.find((c) => c.id === activeId) ?? null,
    watchConversation: vi.fn(() => vi.fn()),
    createNew: vi.fn(),
    select: setActiveId,
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
    missingConversationIds: new Set(missing),
  }))
  return <TooltipProvider>{children()}</TooltipProvider>
}

/** One workspace tab's panes; only the active tab is mounted, as in App. */
function Workspace({
  tab,
  panes = ['pane-1'],
  initial = 'chat-a',
  missing,
}: {
  tab: string
  panes?: string[]
  initial?: string
  missing?: string[]
}) {
  return (
    <Store initial={initial} missing={missing}>
      {() =>
        panes.map((pane) => (
          <section key={`${tab}:${pane}`} data-pane={`${tab}:${pane}`}>
            <ChatPanel density="dock" paneKey={chatPaneKey(tab, pane)} />
          </section>
        ))
      }
    </Store>
  )
}

let root: Root | null = null
let host: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  localStorage.clear()
  sessionStorage.clear()
  resetChatPanesForTest()
  host = document.body.appendChild(document.createElement('div'))
  root = createRoot(host)
})

afterEach(async () => {
  if (root) await act(async () => root?.unmount())
  root = null
  host.remove()
})

const render = (node: React.ReactNode) => act(async () => root?.render(node))

/** The chat the pane shows. */
const shownIn = (pane: string) =>
  host
    .querySelector(`[data-pane="${pane}"] [data-chat-conversation-id]`)
    ?.getAttribute('data-chat-conversation-id')

async function pick(pane: string, id: string) {
  const button = host.querySelector<HTMLButtonElement>(
    `[data-pane="${pane}"] [data-pick="${id}"]`,
  )
  await act(async () => {
    button?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    button?.click()
  })
}

describe('ChatPanel in workspace panes', () => {
  it('keeps each workspace tab on its own chat', async () => {
    // One store across tab switches: re-rendering Workspace keeps its state.
    const tabs = (tab: string) => <Workspace tab={tab} />
    await render(tabs('tab-a'))
    expect(shownIn('tab-a:pane-1')).toBe('chat-a')

    await render(tabs('tab-b'))
    await pick('tab-b:pane-1', 'chat-b')
    expect(shownIn('tab-b:pane-1')).toBe('chat-b')

    await render(tabs('tab-a'))
    expect(shownIn('tab-a:pane-1')).toBe('chat-a')
    // The tab switched to hands its chat back: it is the active one again.
    expect(globalActive).toBe('chat-a')

    await render(tabs('tab-b'))
    expect(shownIn('tab-b:pane-1')).toBe('chat-b')
  })

  it('keeps two panes in one tab on their own chats', async () => {
    await render(<Workspace tab="tab-a" panes={['pane-1', 'pane-2']} />)
    await pick('tab-a:pane-2', 'chat-b')
    expect(shownIn('tab-a:pane-2')).toBe('chat-b')
    expect(shownIn('tab-a:pane-1')).toBe('chat-a')

    await pick('tab-a:pane-1', 'chat-c')
    expect(shownIn('tab-a:pane-1')).toBe('chat-c')
    expect(shownIn('tab-a:pane-2')).toBe('chat-b')
  })

  it('remembers a pane’s chat across a reload', async () => {
    await render(<Workspace tab="tab-a" />)
    await pick('tab-a:pane-1', 'chat-c')
    await act(async () => root?.unmount())
    resetChatPanesForTest()
    root = createRoot(host)

    await render(<Workspace tab="tab-a" initial="chat-a" />)
    expect(shownIn('tab-a:pane-1')).toBe('chat-c')
    expect(globalActive).toBe('chat-c')
  })

  it('shows a chat an outside request opened the pane for', async () => {
    await render(<Workspace tab="tab-a" />)
    await pick('tab-a:pane-1', 'chat-b')
    await act(async () => root?.unmount())
    root = createRoot(host)

    // No chat pane on screen: the request waits for the pane it opens.
    requestChatShown('chat-c')
    await render(<Workspace tab="tab-a" initial="chat-c" />)
    expect(shownIn('tab-a:pane-1')).toBe('chat-c')
  })

  it('lets the lead pane follow a chat picked from outside', async () => {
    await render(<Workspace tab="tab-a" panes={['pane-1', 'pane-2']} />)
    await pick('tab-a:pane-2', 'chat-b')
    // The palette, a page that started a turn: the store's active chat moves.
    await act(async () => {
      requestChatShown('chat-c')
      mocks.useConversationsCtx().select('chat-c')
    })
    expect(shownIn('tab-a:pane-2')).toBe('chat-c')
    expect(shownIn('tab-a:pane-1')).toBe('chat-a')
  })

  it('never hands back a remembered chat the store does not know', async () => {
    // An unsent draft from before a reload, or another stack's chat.
    savePaneChat(chatPaneKey('tab-a', 'pane-1'), 'chat-gone')
    await render(<Workspace tab="tab-a" initial="chat-a" />)
    // Still resolving: the pane shows the active chat and leaves it active.
    expect(shownIn('tab-a:pane-1')).toBe('chat-a')
    expect(globalActive).toBe('chat-a')
  })

  it('lets go of a remembered chat once it is reported missing', async () => {
    savePaneChat(chatPaneKey('tab-a', 'pane-1'), 'chat-gone')
    await render(
      <Workspace tab="tab-a" initial="chat-b" missing={['chat-gone']} />,
    )
    expect(shownIn('tab-a:pane-1')).toBe('chat-b')
    expect(globalActive).toBe('chat-b')
    expect(localStorage.getItem('console:chat:pane-chats')).toContain('chat-b')
  })

  it('keeps this browser tab’s chat over another tab’s pick on reload', async () => {
    await render(<Workspace tab="tab-a" />)
    await pick('tab-a:pane-1', 'chat-c')
    await act(async () => root?.unmount())
    resetChatPanesForTest()
    root = createRoot(host)

    // Another browser tab shares localStorage and picked chat-b last.
    localStorage.setItem(
      'console:chat:pane-chats',
      JSON.stringify({ [chatPaneKey('tab-a', 'pane-1')]: 'chat-b' }),
    )
    await render(<Workspace tab="tab-a" initial="chat-b" />)
    expect(shownIn('tab-a:pane-1')).toBe('chat-c')
    expect(globalActive).toBe('chat-c')
  })

  it('seeds a freshly opened browser tab from the last pick anywhere', async () => {
    localStorage.setItem(
      'console:chat:pane-chats',
      JSON.stringify({ [chatPaneKey('tab-a', 'pane-1')]: 'chat-b' }),
    )
    await render(<Workspace tab="tab-a" initial="chat-a" />)
    expect(shownIn('tab-a:pane-1')).toBe('chat-b')
  })
})
