// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import { TowerScreen } from '.'
import { towerFixture } from './fixtures'
import type { QuestionNeed } from './model'

const mocks = vi.hoisted(() => ({
  ctx: {} as Record<string, unknown>,
  create: vi.fn(),
  chatProps: null as Record<string, unknown> | null,
}))
vi.mock('@/lib/conversations-context', () => ({
  useConversationsCtxOptional: () => mocks.ctx,
}))
vi.mock('./side-chat', async (original) => ({
  ...(await original<object>()),
  createDecisionSideChat: mocks.create,
}))
vi.mock('@/components/chat/ChatView', () => ({
  ChatView: (props: Record<string, unknown>) => {
    mocks.chatProps = props
    return <div data-fixture-chat="">Side chat reply</div>
  },
}))

;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true
const question = towerFixture.items[0] as QuestionNeed
const meta: SessionMeta = {
  session_id: 'copy',
  title: 'Decision copy',
  description: '',
  status: 'idle',
  forked_from: question.sessionId,
  created_at: 1,
  updated_at: 1,
  message_count: 1,
  metadata: {
    side_chat_of: question.sessionId,
    side_chat_question: `${question.sessionId}/${question.questionId}`,
    side_chat_question_kind: 'question',
  },
}
let root: Root
let container: HTMLDivElement
let call: ReturnType<typeof vi.fn>
beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  call = vi.fn(async (id: string, input: Record<string, unknown>) => {
    if (id === 'session::get') return { meta }
    if (id === 'session::set-meta')
      return { meta: { ...meta, metadata: input.metadata } }
    return {
      ...question.entry,
      state: 'answered',
      outcome: 'answered',
      by: { channel: 'tower' },
    }
  })
  mocks.ctx = {
    backend: {
      id: 'fixture',
      contextUsage: vi.fn(async () => ({
        total: 182000,
        usable: 200000,
        free: 18000,
      })),
    },
    conversations: [
      {
        id: 'copy',
        title: 'Decision copy',
        createdAt: 1,
        updatedAt: 1,
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
    setSessionMetadata: vi.fn(),
    modelOptions: [],
  }
  mocks.create.mockResolvedValue({
    meta,
    quote: { messageId: 'pending', text: 'Pending question seed', full: true },
    fresh: true,
  })
})
afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.unstubAllGlobals()
})
async function render() {
  await act(() =>
    root.render(
      <TooltipProvider>
        <TowerScreen
          data={towerFixture}
          scope="all"
          onScopeChange={() => {}}
          initialView="needs-you"
          iii={{ call: call as IiiClient['call'] }}
        />
      </TooltipProvider>,
    ),
  )
}
async function click(prefix: string) {
  const button = [...container.querySelectorAll('button')].find((node) =>
    node.textContent?.trim().startsWith(prefix),
  )
  expect(button, prefix).toBeDefined()
  await act(() => button?.click())
}
describe('decision side chat integration', () => {
  it.each(['option', 'text'])(
    'fills an %s, and only Answer sends to the original session',
    async (kind) => {
      const proposal =
        kind === 'option'
          ? question.entry.questions[0].options[1].label
          : 'Use a dedicated build machine.'
      const conversations = mocks.ctx.conversations as Array<{
        messages: Array<{ content: string }>
      }>
      conversations[0].messages[0].content = proposal
      await render()
      expect(container.textContent).toContain('182k')
      await click('Ask about this')
      expect(mocks.create).toHaveBeenCalledOnce()
      expect(
        (mocks.chatProps?.side as { quote: unknown } | undefined)?.quote,
      ).toMatchObject({ full: true })
      await click('Use this answer')
      expect(call).not.toHaveBeenCalledWith(
        'harness::ask::answer',
        expect.anything(),
      )
      if (kind === 'option')
        expect(
          [
            ...container.querySelectorAll<HTMLInputElement>('input:checked'),
          ].map((node) => node.closest('label')?.textContent),
        ).toContainEqual(expect.stringContaining(proposal))
      else
        expect(
          container.querySelector<HTMLInputElement>(
            '.tower-answer-field input:not([type="radio"])',
          )?.value,
        ).toBe(proposal)
      await click('Answer')
      expect(call).toHaveBeenCalledWith('harness::ask::answer', {
        session_id: question.sessionId,
        question_id: question.questionId,
        answers: [
          {
            header: 'Builds',
            ...(kind === 'option' ? { choice: proposal } : { other: proposal }),
          },
        ],
        by: { channel: 'tower', user: 'user' },
      })
      expect(container.querySelector('.tower-side-chat')).toBeNull()
    },
  )
  it('serializes Keep without dropping its harness ceiling or original-session tag', async () => {
    let release = () => {}
    const ready = new Promise<void>((resolve) => {
      release = resolve
    })
    const request = vi.fn(async (_name: string, run: () => Promise<void>) => {
      await ready
      return run()
    })
    vi.stubGlobal('navigator', { locks: { request } })
    await render()
    await click('Ask about this')
    await click('Keep this side chat')
    expect(request).toHaveBeenCalledWith(
      'iii-decision-side-chat-lifetime:copy',
      expect.any(Function),
    )
    expect(call).not.toHaveBeenCalledWith('session::get', expect.anything())
    await act(async () => {
      release()
      await ready
    })
    expect(call).toHaveBeenCalledWith('session::set-meta', {
      session_id: 'copy',
      metadata: { ...meta.metadata, side_chat_keep: true },
    })
    expect(container.textContent).toContain('Kept in your chat list')
    expect(call).not.toHaveBeenCalledWith('session::delete', expect.anything())
  })
})
