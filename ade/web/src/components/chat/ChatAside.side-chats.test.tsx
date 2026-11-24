// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ContextItems } from '@/lib/backend/context-items'
import type { SideChatRow } from '@/lib/chat-aside'
import { onComposerInsert, registerComposer } from '@/lib/composer-insert'
import type { Conversation } from '@/types/chat'
import type { AsideRequest } from './ChatAside'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  ctx: null as Record<string, unknown> | null,
  call: vi.fn(),
  setSessionDraft: vi.fn(),
}))

vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtx: () => mocks.ctx,
}))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ call: mocks.call }),
}))
vi.mock('@/lib/sessions/api', () => ({
  setSessionMeta: vi.fn(),
  deleteSession: vi.fn(),
  setSessionDraft: mocks.setSessionDraft,
}))
vi.mock('./ChatView', () => ({
  ChatView: ({
    conversation,
    side,
  }: {
    conversation: Conversation
    side?: {
      locked?: { label: string } | null
      placeholder?: string
      onBringBack?: (text: string) => void
      bringBackLabel?: string
      summaries?: readonly string[]
    }
  }) => (
    <div
      data-side-chat={conversation.id}
      data-locked={side?.locked?.label ?? ''}
      data-placeholder={side?.placeholder ?? ''}
      data-summaries={(side?.summaries ?? []).join(',')}
    >
      <button type="button" onClick={() => side?.onBringBack?.('Keep five.')}>
        {side?.bringBackLabel ?? 'no bring back'}
      </button>
    </div>
  ),
}))
vi.mock('./ChatContextPanel', () => ({
  ChatContextPanel: ({
    sideChats,
  }: {
    sideChats?: {
      rows: readonly SideChatRow[]
      onOpen: (id: string) => void
      onNew: () => void
      onKeep: (id: string) => void
      onDelete: (id: string) => void
    }
  }) => (
    <div data-context-panel>
      {sideChats ? (
        <>
          <button type="button" onClick={sideChats.onNew}>
            New
          </button>
          {sideChats.rows.map((row) => (
            <div key={row.id} data-row={row.id}>
              <span data-row-meta>
                {row.title} · {row.messageCount}
                {row.running ? ' · running' : ''}
              </span>
              <button type="button" onClick={() => sideChats.onOpen(row.id)}>
                {`Open ${row.id}`}
              </button>
              <button type="button" onClick={() => sideChats.onKeep(row.id)}>
                {`Keep ${row.id}`}
              </button>
            </div>
          ))}
        </>
      ) : null}
    </div>
  ),
}))

import { ChatAside } from './ChatAside'

function chat(id: string, extra: Partial<Conversation> = {}): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    hydrated: true,
    createdAt: 10,
    updatedAt: 10,
    ...extra,
  }
}

const main = chat('main', {
  title: 'Webhook retries',
  started: true,
  messages: [{ id: 'e1', role: 'assistant', content: 'Five.', createdAt: 1 }],
})
const lockMeta = {
  side_chat_of: 'main',
  side_chat_lock: {
    model: 'claude-opus-5-5',
    provider: 'anthropic',
    thinking_level: 'high',
  },
}
const why = chat('why', {
  title: 'Why five?',
  createdAt: 20,
  updatedAt: 40,
  messages: [
    { id: 'w1', role: 'user', content: 'Why five?', createdAt: 25 },
    { id: 'w2', role: 'assistant', content: 'Eight outlasts.', createdAt: 26 },
  ],
  sessionMetadata: { ...lockMeta, side_chat_summaries: ['e-sum'] },
})
const logs = chat('logs', {
  title: 'Log each attempt?',
  createdAt: 30,
  updatedAt: 50,
  status: 'working',
  sessionMetadata: lockMeta,
})

const items = (rows: ContextItems['side_chats']): ContextItems => ({
  session_id: 'main',
  message_count: 2,
  generated_at_ms: 0,
  subagents: [],
  pull_requests: [],
  links: [],
  attachments: [],
  images: [],
  side_chats: rows,
  truncated: {},
})

function makeCtx(conversations: Conversation[]) {
  return {
    conversations,
    connectionState: 'connected',
    watchConversation: vi.fn(() => () => {}),
    setDraftText: vi.fn(),
    getDraftText: vi.fn(() => 'Also log it.' as string | undefined),
    setSessionMetadata: vi.fn(),
    remove: vi.fn(() => Promise.resolve()),
    openConversationInPanel: vi.fn(),
    backend: { id: 'real' },
    modelOptions: [
      { id: 'anthropic::claude-opus-5-5', label: 'Claude Opus 5.5' },
    ],
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
const changed = vi.fn()
let engineItems: ContextItems | null = null

function render(request: AsideRequest | null = null, phone = false) {
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
          phone={phone}
          sheetOpen={phone}
          onSheetOpenChange={() => {}}
          request={request}
          observers={[]}
          onDisposeObserver={() => {}}
          pendingQuote={null}
          onPendingQuoteChange={() => {}}
          onJumpToMessage={() => {}}
          contextItems={engineItems}
          engineSideChats
          onSideChatsChanged={changed}
        />
      </TooltipProvider>,
    ),
  )
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
  mocks.call.mockReset()
  mocks.setSessionDraft.mockReset()
  mocks.setSessionDraft.mockResolvedValue({ draft: null, attachments: [] })
  changed.mockReset()
  engineItems = items([
    {
      session_id: 'logs',
      title: 'Log each attempt?',
      message_count: 2,
      created_at_ms: 30,
      updated_at_ms: 50,
      from_entry_id: 'e1',
      status: 'running',
      running: true,
    },
    {
      session_id: 'why',
      title: 'Why five?',
      message_count: 6,
      created_at_ms: 20,
      updated_at_ms: 40,
      from_entry_id: 'e1',
      status: 'done',
      running: false,
    },
  ])
  mocks.ctx = makeCtx([main, why, logs])
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
  document.body.innerHTML = ''
})

describe('ChatAside with the engine side chats', () => {
  it('lists several side chats of the conversation with their counts', () => {
    render()
    const rows = [...document.querySelectorAll('[data-row]')]
    expect(rows.map((row) => row.getAttribute('data-row'))).toEqual([
      'logs',
      'why',
    ])
    expect(rows[0].textContent).toContain('Log each attempt? · 2 · running')
    expect(rows[1].textContent).toContain('Why five? · 6')
    // No Side chat tab: the pane is the Context, side chats inside it.
    expect(document.querySelector('[role="tab"]')).toBeNull()
    expect(document.querySelector('[data-context-heading]')).not.toBeNull()
  })

  it('opens one in the side-chat view with its locked model and "Ask aside…"', async () => {
    render()
    act(() => button('Open why')?.click())
    const view = document.querySelector('[data-side-chat="why"]')
    expect(view?.getAttribute('data-locked')).toBe('Claude Opus 5.5 · High')
    expect(view?.getAttribute('data-placeholder')).toBe('Ask aside…')
    expect(view?.getAttribute('data-summaries')).toBe('e-sum')
    expect(
      document.querySelector('[data-side-chat-header]')?.textContent,
    ).toContain('Why five?')
    // Back returns to the list; one with messages is not closed.
    act(() => button('Back to context')?.click())
    await settle()
    expect(document.querySelector('[data-side-chat]')).toBeNull()
    expect(mocks.call).not.toHaveBeenCalled()
  })

  it('names the default effort when the lock carries none (D97)', () => {
    const plain = chat('plain', {
      title: 'Why jitter?',
      createdAt: 25,
      updatedAt: 45,
      messages: [
        { id: 'p1', role: 'user', content: 'Why jitter?', createdAt: 30 },
      ],
      sessionMetadata: {
        side_chat_of: 'main',
        side_chat_lock: { model: 'claude-opus-5-5', provider: 'anthropic' },
      },
    })
    mocks.ctx = makeCtx([main, why, logs, plain])
    render({ seq: 1, tab: 'side-chat', openId: 'plain' })
    expect(
      document
        .querySelector('[data-side-chat="plain"]')
        ?.getAttribute('data-locked'),
    ).toBe('Claude Opus 5.5 · Default')
  })

  it('opens a new one through harness::side-chat::open and closes it when left empty', async () => {
    const fresh = chat('fresh', {
      title: 'Side chat',
      createdAt: 60,
      updatedAt: 60,
      sessionMetadata: lockMeta,
    })
    mocks.call.mockImplementation((fn: string) => {
      if (fn === 'harness::side-chat::open') {
        mocks.ctx = makeCtx([main, why, logs, fresh])
        return Promise.resolve({ session_id: 'fresh', meta: {} })
      }
      if (fn === 'harness::side-chat::close') {
        return Promise.resolve({ deleted: true, operation_id: 'op' })
      }
      return Promise.reject(new Error(`unexpected ${fn}`))
    })
    render()
    act(() => button('New')?.click())
    await settle()
    render()
    expect(mocks.call).toHaveBeenCalledWith('harness::side-chat::open', {
      session_id: 'main',
    })
    expect(document.querySelector('[data-side-chat="fresh"]')).not.toBeNull()
    act(() => button('Back to context')?.click())
    await settle()
    expect(mocks.call).toHaveBeenCalledWith('harness::side-chat::close', {
      session_id: 'fresh',
    })
    expect(changed).toHaveBeenCalled()
  })

  it('forks at the reply a "Side chat" action names', async () => {
    mocks.call.mockResolvedValue({ session_id: 'why', meta: {} })
    render({ seq: 1, tab: 'side-chat', fork: { entryId: 'e1' } })
    await settle()
    expect(mocks.call).toHaveBeenCalledWith('harness::side-chat::open', {
      session_id: 'main',
      entry_id: 'e1',
    })
  })

  it('keeps one as a conversation through harness::side-chat::keep', async () => {
    mocks.call.mockResolvedValue({ meta: {} })
    render()
    act(() => button('Keep why')?.click())
    await settle()
    expect(mocks.call).toHaveBeenCalledWith('harness::side-chat::keep', {
      session_id: 'why',
    })
    expect(document.querySelector('[data-row="why"]')).toBeNull()
    expect(document.body.textContent).toContain('Kept as a conversation')
  })

  it('brings a reply back into the parent draft with session::set-draft', async () => {
    render()
    act(() => button('Open why')?.click())
    act(() => button('Bring back')?.click())
    await settle()
    const text = 'Also log it.\n\nFrom side chat “Why five?”:\n\nKeep five.'
    expect(mocks.setSessionDraft).toHaveBeenCalledWith('main', text)
    expect(
      (mocks.ctx as ReturnType<typeof makeCtx>).setDraftText,
    ).toHaveBeenCalledWith('main', text)
    expect(
      document.querySelector('[data-side-chat-toast]')?.textContent,
    ).toContain('Added to the Webhook retries composer')
  })

  it('hands the reply to the conversation composer when it is mounted', () => {
    const off = registerComposer('main-composer', 'main')
    const inserts: string[] = []
    const stop = onComposerInsert(
      (insert) => inserts.push(insert.text),
      'main-composer',
    )
    try {
      render()
      act(() => button('Open why')?.click())
      act(() => button('Bring back')?.click())
      // After the draft's text ("Also log it."): a blank line, the
      // header, a blank line, the reply — one paragraph per line.
      expect(inserts).toEqual([
        '',
        'From side chat “Why five?”:',
        '',
        'Keep five.',
      ])
      expect(mocks.setSessionDraft).not.toHaveBeenCalled()
    } finally {
      stop()
      off()
    }
  })

  it('labels Bring back with the conversation on a phone, and opens there as a page', () => {
    render({ seq: 1, tab: 'side-chat', openId: 'why' }, true)
    expect(document.querySelector('[data-side-chat-page]')).not.toBeNull()
    expect(button('Bring back to Webhook retries')).toBeDefined()
  })
})
