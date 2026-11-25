// @vitest-environment jsdom

import { act, StrictMode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '@/types/chat'
import { MessageList } from './MessageList'

const ask: Message = {
  id: 'fc_ask',
  role: 'function-call',
  functionCallId: 'call-ask',
  functionId: 'harness::ask',
  input: {},
  output: {
    content: [{ type: 'text', text: 'awaiting answer' }],
    details: { status: 'awaiting_answer', question_id: 'q1' },
  },
  createdAt: 1,
} as Message
const reply: Message = {
  id: 'a1',
  role: 'assistant',
  content: 'I asked above.',
  createdAt: 2,
} as Message

let container: HTMLDivElement
let root: Root

const wait = (ms: number) =>
  act(() => new Promise<void>((resolve) => setTimeout(resolve, ms)))

/** Lay the list out: the viewport 0–600, the card at `cardTop`. */
function layout(cardTop: number) {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(
    function (this: Element) {
      const el = this as HTMLElement
      if (el.dataset.messageList !== undefined) {
        return DOMRect.fromRect({ x: 0, y: 0, width: 800, height: 600 })
      }
      if (el.dataset.messageId === 'fc_ask') {
        return DOMRect.fromRect({ x: 0, y: cardTop, width: 700, height: 200 })
      }
      return DOMRect.fromRect({ x: 0, y: 0, width: 0, height: 0 })
    },
  )
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  // jsdom has no smooth scrolling.
  Element.prototype.scrollTo ??= () => {}
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(async () => {
  await act(() => root.unmount())
  container.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

let reported: { count: number; direction: string; jump: () => void } | null =
  null

async function render(awaitingQuestionIds: readonly string[]) {
  reported = null
  await act(() =>
    root.render(
      // StrictMode, as the app runs in dev: its remount once left the
      // measuring frame stuck and nothing was ever reported.
      <StrictMode>
        <MessageList
          messages={[ask, reply]}
          awaitingQuestionIds={awaitingQuestionIds}
          onQuestionsOffscreen={(offscreen, jump) => {
            reported = { ...offscreen, jump }
          }}
        />
      </StrictMode>,
    ),
  )
  // The opening veil lifts once layout settles, then the list measures on
  // the next frame.
  for (let i = 0; i < 40; i++) {
    const list = container.querySelector('[data-message-list]')
    if (list && !list.hasAttribute('data-transcript-opening')) break
    await wait(50)
  }
  await wait(50)
}

describe('waiting question cards out of view', () => {
  it('reports a waiting card scrolled out of view, and the jump lands on it', async () => {
    layout(-900)
    await render(['fc_ask'])
    expect(reported).toMatchObject({ count: 1, direction: 'up' })
    // The list draws no pill of its own over the transcript.
    expect(container.querySelector('[data-question-jump]')).toBeNull()

    await act(() => reported?.jump())
    const card = container.querySelector('[data-message-id="fc_ask"]')
    expect(card?.contains(document.activeElement)).toBe(true)
  })

  it('reports nothing once the card scrolls into view', async () => {
    layout(-900)
    await render(['fc_ask'])
    expect(reported?.count).toBe(1)

    vi.restoreAllMocks()
    layout(200)
    const list = container.querySelector('[data-message-list]')
    await act(() => list?.dispatchEvent(new Event('scroll')))
    await wait(50)
    expect(reported?.count).toBe(0)
  })

  it('reports nothing for a card that no longer waits', async () => {
    layout(-900)
    await render([])
    expect(reported?.count ?? 0).toBe(0)
  })

  it('keeps a waiting card visible inside a folded batch, so the jump can land on it', async () => {
    // The ask is not the batch's last call and no renderer marks it as a
    // display, so a folded batch would otherwise hide it.
    const later: Message = {
      id: 'fc_later',
      role: 'function-call',
      functionCallId: 'call-later',
      functionId: 'engine::functions::list',
      input: {},
      output: { functions: [] },
      createdAt: 2,
    } as Message
    const view = (awaitingQuestionIds: readonly string[]) => (
      <MessageList
        messages={[ask, later, reply]}
        awaitingQuestionIds={awaitingQuestionIds}
      />
    )
    const askCard = '[data-message-id="fc_ask"]'
    await act(() => root.render(view([])))
    expect(container.querySelector(askCard)).toBeNull()
    await act(() => root.render(view(['fc_ask'])))
    expect(container.querySelector(askCard)).not.toBeNull()
  })
})
