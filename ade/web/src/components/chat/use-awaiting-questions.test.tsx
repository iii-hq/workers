// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '@/types/chat'
import {
  type AwaitingQuestion,
  type AwaitingQuestionsOptions,
  useAwaitingQuestions,
} from './use-awaiting-questions'

const ask = (id: string, questionId: string) =>
  ({
    id,
    role: 'function-call',
    functionId: 'harness::ask',
    input: {},
    output: {
      content: [{ type: 'text', text: 'awaiting answer' }],
      details: { status: 'awaiting_answer', question_id: questionId },
    },
    createdAt: 1,
  }) as Message

let container: HTMLDivElement
let root: Root
let seen: readonly AwaitingQuestion[] = []

function Probe(props: AwaitingQuestionsOptions) {
  seen = useAwaitingQuestions(props)
  return null
}

const flush = () =>
  act(() => new Promise<void>((resolve) => setTimeout(resolve, 0)))

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  root = createRoot(container)
  seen = []
})

afterEach(async () => {
  await act(() => root.unmount())
  vi.unstubAllGlobals()
})

const user = (id: string) =>
  ({ id, role: 'user', content: 'reply', createdAt: 2 }) as Message

describe('useAwaitingQuestions', () => {
  it('follows what the harness knows, and re-reads on ask events', async () => {
    let states = new Map([
      ['q1', true],
      ['q2', true],
      ['q3', false],
    ])
    let ring = () => {}
    const askStates = vi.fn(async () => states)
    const off = vi.fn()
    const onAskChanged = vi.fn((_: string, onEvent: () => void) => {
      ring = onEvent
      return off
    })
    // q1 waits even with a message after it: the harness says it is open.
    const messages = [
      ask('fc_1', 'q1'),
      user('u1'),
      ask('fc_2', 'q2'),
      ask('fc_3', 'q3'),
    ]
    await act(() =>
      root.render(
        <Probe
          backend={{ askStates, onAskChanged }}
          sessionId="s1"
          messages={messages}
          waitingOnAnswer={false}
        />,
      ),
    )
    await flush()
    expect(askStates).toHaveBeenCalledWith('s1')
    expect(seen.map((q) => q.messageId)).toEqual(['fc_1', 'fc_2'])

    // Answered in another tab: the settled event re-reads.
    states = new Map([
      ['q1', false],
      ['q2', true],
      ['q3', false],
    ])
    ring()
    await flush()
    expect(seen.map((q) => q.messageId)).toEqual(['fc_2'])

    await act(() => root.unmount())
    expect(off).toHaveBeenCalled()
    root = createRoot(container)
  })

  it('a card the harness does not know waits until the person says something after it', async () => {
    const askStates = vi.fn(async () => new Map<string, boolean>())
    const view = (messages: Message[]) => (
      <Probe
        backend={{ askStates }}
        sessionId="s1"
        messages={messages}
        waitingOnAnswer={false}
      />
    )
    await act(() => root.render(view([ask('fc_1', 'q1'), ask('fc_2', 'q2')])))
    await flush()
    expect(seen.map((q) => q.messageId)).toEqual(['fc_1', 'fc_2'])

    await act(() =>
      root.render(view([ask('fc_1', 'q1'), user('u1'), ask('fc_2', 'q2')])),
    )
    await flush()
    expect(seen.map((q) => q.messageId)).toEqual(['fc_2'])
  })

  it('a failed read falls back to the same rule', async () => {
    const askStates = vi.fn(async () => {
      throw new Error('function_not_found')
    })
    await act(() =>
      root.render(
        <Probe
          backend={{ askStates }}
          sessionId="s1"
          messages={[ask('fc_1', 'q1'), user('u1'), ask('fc_2', 'q2')]}
          waitingOnAnswer
        />,
      ),
    )
    await flush()
    expect(seen.map((q) => q.messageId)).toEqual(['fc_2'])
  })

  it('reads nothing while the chat has no question card', async () => {
    const askStates = vi.fn(async () => new Map<string, boolean>())
    await act(() =>
      root.render(
        <Probe
          backend={{ askStates }}
          sessionId="s1"
          messages={[user('u1')]}
          waitingOnAnswer={false}
        />,
      ),
    )
    await flush()
    expect(askStates).not.toHaveBeenCalled()
    expect(seen.map((q) => q.messageId)).toEqual([])
  })
})
