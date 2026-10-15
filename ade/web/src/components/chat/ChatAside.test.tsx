// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { type SideChatQuote, sideChatsMadeHere } from '@/lib/chat-aside'
import type { Conversation } from '@/types/chat'
import type { AsideRequest } from './ChatAside'

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const mocks = vi.hoisted(() => ({
  ctx: null as Record<string, unknown> | null,
  trigger: vi.fn(),
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
  setSessionMeta: mocks.setSessionMeta,
  deleteSession: mocks.deleteSession,
}))
vi.mock('./ChatView', () => ({
  ChatView: ({
    conversation,
    side,
  }: {
    conversation: Conversation
    side?: {
      autoFocus?: boolean
      quote?: SideChatQuote | null
      onQuoteSent?: (quote: SideChatQuote) => void
    }
  }) => (
    <div
      data-side-chat={conversation.id}
      data-autofocus={String(side?.autoFocus ?? false)}
      data-quote={side?.quote?.text ?? ''}
    >
      <button
        type="button"
        onClick={() => side?.quote && side.onQuoteSent?.(side.quote)}
      >
        send quote
      </button>
    </div>
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

const pending = vi.fn()
let pendingQuote: SideChatQuote | null = null

function render(
  request: AsideRequest | null = null,
  conversation: Conversation = main,
  onAskAgent?: (text: string, send: boolean) => void,
) {
  if (!container) {
    container = document.createElement('div')
    document.body.appendChild(container)
    root = createRoot(container)
  }
  act(() =>
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ChatAside
          conversation={conversation}
          side="right"
          phone={false}
          sheetOpen={false}
          onSheetOpenChange={() => {}}
          request={request}
          triggers={[]}
          onUnregisterTrigger={() => {}}
          pendingQuote={pendingQuote}
          onPendingQuoteChange={pending}
          onJumpToMessage={() => {}}
          onAskAgent={onAskAgent}
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

/** The session worker: tails for the main chat and the fork, and the fork. */
function route(fork: () => Promise<unknown>) {
  mocks.trigger.mockImplementation(
    (fn: string, payload: Record<string, unknown>) => {
      if (fn === 'session::fork') return fork()
      if (fn === 'session::messages-tail') {
        return Promise.resolve(
          payload.session_id === 'main'
            ? {
                messages: [
                  { entry_id: 'e0', message: { role: 'user' } },
                  { entry_id: 'e1', message: { role: 'assistant' } },
                  { entry_id: 'e2', message: { role: 'function_result' } },
                ],
              }
            : {
                messages: [{ entry_id: 'f1', message: { role: 'assistant' } }],
              },
        )
      }
      return Promise.reject(new Error(fn))
    },
  )
}

const forked = (metadata: Record<string, unknown> = {}) => ({
  session_id: 'fork-1',
  meta: { created_at: 20, metadata },
})

beforeEach(() => {
  pending.mockReset()
  pendingQuote = null
  window.localStorage.clear()
  sideChatsMadeHere.clear()
  mocks.trigger.mockReset()
  mocks.setSessionMeta.mockReset()
  mocks.deleteSession.mockReset()
  mocks.setSessionMeta.mockResolvedValue({})
  mocks.deleteSession.mockResolvedValue({ deleted: true })
})

afterEach(() => {
  act(() => root?.unmount())
  container?.remove()
  root = null
  container = null
})

const forkCalls = () =>
  mocks.trigger.mock.calls.filter(([fn]) => fn === 'session::fork')

describe('ChatAside side chat', () => {
  it('forks once at the last spoken entry, tags, anchors and unlinks it', async () => {
    let resolveFork: (value: unknown) => void = () => {}
    route(
      () =>
        new Promise((resolve) => {
          resolveFork = resolve
        }),
    )
    mocks.ctx = makeCtx([main])
    render({ seq: 1, tab: 'side-chat' })
    render({ seq: 2, tab: 'side-chat' })
    await settle()
    resolveFork(
      forked({
        agent_profile: { id: 'p' },
        parent_session_id: 'boss',
        depth: 1,
      }),
    )
    await settle()

    expect(forkCalls()).toEqual([
      [
        'session::fork',
        { session_id: 'main', entry_id: 'e1', title: 'Side chat · Main' },
      ],
    ])
    expect(mocks.setSessionMeta).toHaveBeenCalledWith({
      session_id: 'fork-1',
      metadata: {
        agent_profile: { id: 'p' },
        side_chat_of: 'main',
        side_chat_from: 'f1',
      },
    })
    expect(sideChatsMadeHere.has('fork-1')).toBe(true)
    // The fork is not in the list yet: no second "Start" to click meanwhile.
    expect(button('Start side chat')).toBeUndefined()

    mocks.ctx = makeCtx([main, fork])
    const view = render({ seq: 2, tab: 'side-chat' })
    expect(view.querySelector('[data-side-chat="fork-1"]')).not.toBeNull()
  })

  it('deletes a fork whose tag could not be written, and says so', async () => {
    route(() => Promise.resolve(forked()))
    mocks.setSessionMeta.mockRejectedValue(new Error('set-meta down'))
    mocks.ctx = makeCtx([main])
    const view = render({ seq: 1, tab: 'side-chat' })
    await settle()
    expect(mocks.deleteSession).toHaveBeenCalledWith('fork-1')
    expect(view.textContent).toContain('Side chat not started')
  })

  it('discards without asking when only copied notes are there', () => {
    const ctx = makeCtx([
      main,
      {
        ...fork,
        messages: [
          {
            id: 'note',
            role: 'system',
            kind: 'model-note',
            tone: 'info',
            content: 'Note to the model',
            createdAt: 99,
          },
        ],
      },
    ])
    mocks.ctx = ctx
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    render()
    act(() => button('Discard side chat')?.click())
    expect(ctx.remove).toHaveBeenCalledWith('fork-1')
  })

  it('hands a quote to the composer, leaves the draft alone and focuses it', async () => {
    const ctx = makeCtx([main, fork])
    mocks.ctx = ctx
    pendingQuote = { messageId: 'm1', text: 'q', agent: 'A' }
    const view = render({ seq: 1, tab: 'side-chat', quote: pendingQuote })
    await settle()
    const sideChat = view.querySelector('[data-side-chat="fork-1"]')
    expect(sideChat?.getAttribute('data-quote')).toBe('q')
    expect(sideChat?.getAttribute('data-autofocus')).toBe('true')
    expect(ctx.setDraftText).not.toHaveBeenCalled()
  })

  it('records a sent quote on the side chat and clears the pending one', () => {
    const ctx = makeCtx([
      main,
      {
        ...fork,
        sessionMetadata: {
          side_chat_of: 'main',
          side_chat_quotes: [{ message_id: 'm0', text: 'older' }],
        },
      },
    ])
    mocks.ctx = ctx
    pendingQuote = { messageId: 'm1', text: 'q', agent: 'A' }
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    render()
    act(() => button('send quote')?.click())
    expect(ctx.setSessionMetadata).toHaveBeenCalledWith('fork-1', {
      side_chat_quotes: [
        { message_id: 'm0', text: 'older' },
        { message_id: 'm1', text: 'q', agent: 'A' },
      ],
    })
    expect(pending).toHaveBeenCalledWith(null)
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
    expect(pending).toHaveBeenCalledWith(null)
    expect(view.querySelector('[data-side-chat]')).toBeNull()
  })

  it('keeps a side chat once the server dropped its tag', async () => {
    mocks.setSessionMeta.mockReset().mockResolvedValue({})
    const ctx = makeCtx([main, fork])
    mocks.ctx = ctx
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    const view = render()
    act(() => button('Keep as conversation')?.click())
    await settle()
    expect(mocks.setSessionMeta).toHaveBeenCalledWith({
      session_id: 'fork-1',
      metadata: {},
    })
    expect(ctx.setSessionMetadata).toHaveBeenCalledWith('fork-1', {
      side_chat_of: undefined,
    })
    expect(view.textContent).toContain('Kept as a conversation')
  })

  it('stays a side chat when the server refuses to keep it', async () => {
    mocks.setSessionMeta.mockReset().mockRejectedValue(new Error('offline'))
    const ctx = makeCtx([main, fork])
    mocks.ctx = ctx
    window.localStorage.setItem('iii-chat-aside-tab', 'side-chat')
    const view = render()
    act(() => button('Keep as conversation')?.click())
    await settle()
    expect(ctx.setSessionMetadata).not.toHaveBeenCalled()
    expect(view.textContent).toContain('Side chat not kept')
    expect(view.textContent).not.toContain('Kept as a conversation')
  })
})

describe('ChatAside unread reply', () => {
  const answered = (count: number) =>
    chat('fork-1', {
      createdAt: 20,
      hydrated: true,
      sessionMetadata: { side_chat_of: 'main' },
      messages: Array.from({ length: count }, (_, i) => ({
        id: `r${i}`,
        role: 'assistant' as const,
        content: 'ok',
        createdAt: 30 + i,
      })),
    })
  const dot = () => document.querySelector('[data-side-chat-unread]')

  it('counts what was there on arrival as read', async () => {
    localStorage.clear()
    mocks.ctx = makeCtx([main, answered(1)])
    render()
    await settle()
    expect(dot()).toBeNull()
  })

  it('counts only a reply as news, not the notes a turn writes first', async () => {
    localStorage.clear()
    mocks.ctx = makeCtx([main, answered(0)])
    render()
    await settle()
    const noted = chat('fork-1', {
      createdAt: 20,
      hydrated: true,
      sessionMetadata: { side_chat_of: 'main' },
      messages: [
        { id: 'n1', role: 'system', content: 'runtime context', createdAt: 30 },
      ] as Conversation['messages'],
    })
    mocks.ctx = makeCtx([main, noted])
    render()
    await settle()
    expect(dot()).toBeNull()
  })

  it('marks the Side chat tab when a reply lands out of sight', async () => {
    localStorage.clear()
    mocks.ctx = makeCtx([main, answered(1)])
    const view = render()
    await settle()
    mocks.ctx = makeCtx([main, answered(2)])
    render()
    await settle()
    expect(dot()?.className).toContain('inline-block')
    expect(button('Side chat, new reply')).toBeDefined()
    const tab = [...view.querySelectorAll('[role="tab"]')].find((el) =>
      el.textContent?.startsWith('Side chat'),
    ) as HTMLElement
    act(() => tab.click())
    await settle()
    expect(dot()).toBeNull()
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

describe('ChatAside auto-fix rounds', () => {
  it('tells the agent what is new on a watched PR, once, and keeps the state', async () => {
    const gh: string[] = []
    mocks.trigger.mockImplementation(
      (fn: string, payload: Record<string, unknown>) => {
        if (fn !== 'shell::exec') return Promise.reject(new Error(fn))
        const args = payload.args as string[]
        gh.push(args.slice(0, 2).join(' '))
        const out = (value: unknown) =>
          Promise.resolve({ exit_code: 0, stdout: JSON.stringify(value) })
        if (args[0] === 'api')
          return Promise.resolve({ exit_code: 0, stdout: 'me\n' })
        if (args[1] === 'view') {
          return out({
            state: 'OPEN',
            mergeable: 'MERGEABLE',
            reviews: [
              { author: { login: 'me' }, state: 'COMMENTED' },
              { author: { login: 'bot' }, state: 'COMMENTED' },
            ],
          })
        }
        return out([
          { name: 'lint', bucket: 'fail' },
          { name: 'test', bucket: 'pass' },
        ])
      },
    )
    const ctx = { ...makeCtx([main]), shellAvailable: true }
    mocks.ctx = ctx
    const ask = vi.fn()
    const watched = chat('main', {
      ...main,
      workingDir: '/work/repo',
      sessionMetadata: {
        context_autofix: {
          'o/r#9': {
            url: 'https://github.com/o/r/pull/9',
            failing: [],
            reviews: 0,
            conflict: false,
            reports: 0,
          },
        },
      },
    })
    render(null, watched, ask)
    await settle()
    expect(ask).toHaveBeenCalledTimes(1)
    expect(ask.mock.calls[0][0]).toContain('https://github.com/o/r/pull/9')
    expect(ask.mock.calls[0][0]).toContain('a check fails: lint')
    expect(ask.mock.calls[0][0]).toContain('a new review')
    expect(ask.mock.calls[0][1]).toBe(true)
    expect(ctx.setSessionMetadata).toHaveBeenCalledWith('main', {
      context_autofix: {
        'o/r#9': {
          url: 'https://github.com/o/r/pull/9',
          failing: ['lint'],
          reviews: 1,
          conflict: false,
          reports: 1,
        },
      },
    })
    expect(gh).toContain('pr checks')
  })

  it('waits while the agent is working', async () => {
    mocks.trigger.mockReset()
    mocks.ctx = { ...makeCtx([main]), shellAvailable: true }
    const ask = vi.fn()
    render(
      null,
      chat('main', {
        ...main,
        status: 'working',
        workingDir: '/work/repo',
        sessionMetadata: {
          context_autofix: {
            'o/r#9': {
              url: 'u',
              failing: [],
              reviews: 0,
              conflict: false,
              reports: 0,
            },
          },
        },
      }),
      ask,
    )
    await settle()
    expect(ask).not.toHaveBeenCalled()
    expect(mocks.trigger).not.toHaveBeenCalled()
  })
})
