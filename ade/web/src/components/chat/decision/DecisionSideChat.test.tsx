// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { IiiClient } from '@/lib/iii-client'
import { TowerScreen } from '@/pages/Tower'
import { towerFixture } from '@/pages/Tower/fixtures'
import type { QuestionNeed } from '@/pages/Tower/model'
import type {
  DecisionChatHandle,
  DecisionChatRequest,
} from '@/types/injectable-ui'
import { DecisionSideChat, DecisionSideChatPanel } from './DecisionSideChat'
import { decisionChatCapabilities, registerDecisionChatWorkspace } from './host'

const mocks = vi.hoisted(() => ({
  ctx: {} as Record<string, unknown>,
  client: { call: vi.fn() },
  create: vi.fn(),
  find: vi.fn(),
}))
vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtxOptional: () => mocks.ctx,
}))
vi.mock('@/lib/iii-client', async (original) => ({
  ...(await original<object>()),
  getIiiClient: async () => mocks.client,
}))
vi.mock('./side-chat', () => ({
  createDecisionSideChat: mocks.create,
  findDecisionSideChat: mocks.find,
}))
vi.mock('@/components/chat/ChatView', () => ({
  ChatView: () => <div>Side chat reply</div>,
}))
vi.mock('@/components/ui/Select', () => ({
  Select: ({
    value,
    options,
    onChange,
    ...props
  }: {
    value: string
    options: Array<{ value: string; label: string }>
    onChange: (value: string) => void
  }) => (
    <select
      {...props}
      value={value}
      onChange={(event) => onChange(event.currentTarget.value)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  ),
}))

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true
const question = towerFixture.items[0] as QuestionNeed
const meta = {
  session_id: 'copy',
  title: 'Decision copy',
  description: '',
  status: 'idle',
  created_at: 1,
  updated_at: 1,
  message_count: 1,
  metadata: {
    side_chat_of: question.sessionId,
    side_chat_question: `${question.sessionId}/${question.questionId}`,
  },
}
let root: Root
let container: HTMLDivElement
let stopWorkspace: (() => void) | undefined

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  mocks.client.call.mockImplementation(
    async (id: string, input: Record<string, unknown>) =>
      id === 'session::get'
        ? { meta }
        : id === 'session::set-meta'
          ? { meta: { ...meta, metadata: input.metadata } }
          : {},
  )
  mocks.ctx = {
    backend: {
      id: 'fixture',
      contextUsage: vi.fn(async () => ({ total: 182000 })),
    },
    conversations: [
      {
        id: 'copy',
        title: 'Decision copy',
        createdAt: 1,
        updatedAt: 1,
        status: 'idle',
        messages: [
          {
            id: 'reply',
            role: 'assistant',
            content: question.entry.questions[0].options[1].label,
            createdAt: 2,
          },
        ],
        sessionMetadata: meta.metadata,
      },
    ],
    watchConversation: vi.fn(() => () => {}),
    modelOptions: [],
  }
  stopWorkspace = registerDecisionChatWorkspace({
    caller: () => ({ tabId: 'tab', paneId: 'tower' }),
    open: vi.fn(),
    close: vi.fn(),
  })
  mocks.create.mockResolvedValue({ meta })
})
afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  stopWorkspace?.()
  vi.unstubAllGlobals()
})
const click = async (label: string) => {
  const button = [...container.querySelectorAll('button')].find((node) =>
    node.textContent?.trim().startsWith(label),
  )
  expect(button, label).toBeDefined()
  await act(() => button?.click())
}

describe('DecisionSideChat', () => {
  it('uses a question proposal at its requested index, disables it after detach, and reenables after rebind', async () => {
    const answer = vi.fn()
    const onClose = vi.fn()
    const capabilities = decisionChatCapabilities(mocks.client)
    const handle = await capabilities.openDecisionChat?.({
      key: `${question.sessionId}/${question.questionId}`,
      sessionId: question.sessionId,
      kind: 'question',
      title: question.title,
      createdAt: new Date(question.createdAt).toISOString(),
      questionId: question.questionId,
      questions: [
        question.entry.questions[0],
        {
          header: 'Second',
          question: 'Second question',
          options: [{ label: 'Second option' }],
        },
      ],
      onUseAnswer: answer,
    })
    await act(() =>
      root.render(
        <TooltipProvider>
          <DecisionSideChat sessionId="copy" onClose={onClose} />
        </TooltipProvider>,
      ),
    )
    expect(container.textContent).toContain('182k')
    const select = container.querySelector('select') as HTMLSelectElement
    await act(() => {
      select.value = '1'
      select.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await click('Use this answer')
    expect(answer).toHaveBeenCalledWith(1, {
      text: question.entry.questions[0].options[1].label,
    })
    await act(() => handle?.detach())
    expect(
      (
        [...container.querySelectorAll('button')].find(
          (node) => node.textContent?.trim() === 'Use this answer',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(true)
    mocks.find.mockResolvedValue(meta)
    const rebound = await capabilities.rebindDecisionChat?.({
      key: `${question.sessionId}/${question.questionId}`,
      sessionId: question.sessionId,
      kind: 'question',
      title: question.title,
      createdAt: new Date(question.createdAt).toISOString(),
      questionId: question.questionId,
      questions: question.entry.questions,
      onUseAnswer: answer,
    })
    await act(() => rebound?.update({ editable: true, closed: false }))
    expect(
      (
        [...container.querySelectorAll('button')].find(
          (node) => node.textContent?.trim() === 'Use this answer',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(false)
    await act(() => rebound?.update({ editable: false, closed: true }))
    expect(
      (
        [...container.querySelectorAll('button')].find(
          (node) => node.textContent?.trim() === 'Use this answer',
        ) as HTMLButtonElement
      ).disabled,
    ).toBe(true)
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('proposes every held-call decision at question index zero', async () => {
    const answer = vi.fn()
    await act(() =>
      root.render(
        <TooltipProvider>
          <DecisionSideChatPanel
            title="run"
            kept={false}
            onKeep={() => {}}
            onClose={() => {}}
            held
            onUseAnswer={answer}
          >
            content
          </DecisionSideChatPanel>
        </TooltipProvider>,
      ),
    )
    for (const option of ['Deny', 'Allow once', 'Always allow']) {
      const select = container.querySelector('select') as HTMLSelectElement
      await act(() => {
        select.value = option
        select.dispatchEvent(new Event('change', { bubbles: true }))
      })
      await click('Use this answer')
    }
    expect(answer.mock.calls).toEqual([
      [0, { option: 'Deny' }],
      [0, { option: 'Allow once' }],
      [0, { option: 'Always allow' }],
    ])
  })

  it('serializes Keep and preserves the decision copy metadata', async () => {
    let release = () => {}
    const ready = new Promise<void>((resolve) => {
      release = resolve
    })
    const lock = vi.fn(async (_name: string, run: () => Promise<void>) => {
      await ready
      return run()
    })
    vi.stubGlobal('navigator', { locks: { request: lock } })
    const capabilities = decisionChatCapabilities(mocks.client)
    await capabilities.openDecisionChat?.({
      key: `${question.sessionId}/${question.questionId}`,
      sessionId: question.sessionId,
      kind: 'question',
      title: question.title,
      createdAt: new Date(question.createdAt).toISOString(),
      questionId: question.questionId,
      questions: question.entry.questions,
      onUseAnswer: () => {},
    })
    await act(() =>
      root.render(
        <TooltipProvider>
          <DecisionSideChat sessionId="copy" onClose={() => {}} />
        </TooltipProvider>,
      ),
    )
    await click('Keep this side chat')
    expect(lock).toHaveBeenCalledWith(
      'iii-decision-side-chat-lifetime:copy',
      expect.any(Function),
    )
    expect(mocks.client.call).not.toHaveBeenCalledWith(
      'session::get',
      expect.anything(),
    )
    await act(async () => {
      release()
      await ready
    })
    expect(mocks.client.call).toHaveBeenCalledWith('session::set-meta', {
      session_id: 'copy',
      metadata: { ...meta.metadata, side_chat_keep: true },
    })
    expect(container.textContent).toContain('Kept in your chat list')
  })
})

describe('Tower decision capability', () => {
  it('detaches a stale rebind that resolves after Ask about this opens the copy', async () => {
    let resolve: (handle: DecisionChatHandle | null) => void = () => {}
    const stale = new Promise<DecisionChatHandle | null>((done) => {
      resolve = done
    })
    const opened: DecisionChatHandle = {
      update: vi.fn(),
      detach: vi.fn(),
      close: vi.fn(),
    }
    const delayed: DecisionChatHandle = {
      update: vi.fn(),
      detach: vi.fn(),
      close: vi.fn(),
    }
    const chat = {
      openDecisionChat: vi.fn(async () => opened),
      rebindDecisionChat: vi.fn(() => stale),
    }
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            data={towerFixture}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            iii={{ call: vi.fn() as IiiClient['call'] }}
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    expect(chat.rebindDecisionChat).toHaveBeenCalled()
    await click('Ask about this')
    expect(chat.openDecisionChat).toHaveBeenCalled()
    await act(async () => {
      resolve(delayed)
      await stale
    })
    expect(delayed.detach).toHaveBeenCalled()
    expect(opened.detach).not.toHaveBeenCalled()
  })

  it('rebinds every displayed decision, closes removed items, and detaches on unmount', async () => {
    const bindings = [
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
      { update: vi.fn(), detach: vi.fn(), close: vi.fn() },
    ]
    const handles = [...bindings]
    const chat = {
      openDecisionChat: vi.fn(),
      rebindDecisionChat: vi.fn(async () => handles.shift() ?? null),
    }
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            data={towerFixture}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })
    expect(chat.rebindDecisionChat).toHaveBeenCalledTimes(
      towerFixture.items.filter((item) => item.kind !== 'stalled').length,
    )
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            data={{ ...towerFixture, items: towerFixture.items.slice(0, 1) }}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
            chat={chat}
          />
        </TooltipProvider>,
      ),
    )
    expect(bindings[1].update).toHaveBeenCalledWith({
      editable: false,
      closed: true,
    })
    await act(() => root.unmount())
    expect(bindings[0].detach).toHaveBeenCalled()
    expect(bindings[1].detach).toHaveBeenCalled()
  })

  it.each(['option', 'text'])(
    'fills a %s through the host request, then only Answer sends it',
    async (kind) => {
      let opened: DecisionChatRequest | undefined
      const chat = {
        openDecisionChat: vi.fn(async (request: DecisionChatRequest) => {
          opened = request
          return { update: vi.fn(), detach: vi.fn(), close: vi.fn() }
        }),
        rebindDecisionChat: vi.fn(async () => null),
      }
      const call = vi.fn(async () => ({ ...question.entry, state: 'answered' }))
      await act(() =>
        root.render(
          <TooltipProvider>
            <TowerScreen
              data={towerFixture}
              scope="all"
              onScopeChange={() => {}}
              initialView="needs-you"
              iii={{ call: call as IiiClient['call'] }}
              chat={chat}
            />
          </TooltipProvider>,
        ),
      )
      await click('Ask about this')
      const proposal =
        kind === 'option'
          ? { option: question.entry.questions[0].options[1].label }
          : { text: 'Use a dedicated build machine.' }
      await act(() => opened?.onUseAnswer(0, proposal))
      await click('Answer')
      expect(call).toHaveBeenCalledWith(
        'harness::ask::answer',
        expect.objectContaining({
          answers: [
            {
              header: 'Builds',
              ...(kind === 'option'
                ? { choice: question.entry.questions[0].options[1].label }
                : { other: 'Use a dedicated build machine.' }),
            },
          ],
        }),
      )
    },
  )

  it('hides Ask about this without the capability', async () => {
    await act(() =>
      root.render(
        <TooltipProvider>
          <TowerScreen
            data={towerFixture}
            scope="all"
            onScopeChange={() => {}}
            initialView="needs-you"
          />
        </TooltipProvider>,
      ),
    )
    expect(container.textContent).not.toContain('Ask about this')
  })
})
