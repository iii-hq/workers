import { describe, expect, it } from 'vitest'
import type { Conversation, Message } from '@/types/chat'
import {
  endsOnQuestion,
  hasWorkingConversation,
  isWaitingOnAnswer,
} from './chat-activity'

describe('chat screen activity', () => {
  it('includes working sessions/subagents even when another panel is selected', () => {
    expect(
      hasWorkingConversation([{ status: 'done' }, { status: 'working' }], true),
    ).toBe(true)
  })

  it.each(['idle', 'done', 'error', undefined] as const)(
    'does not hold the screen for %s sessions',
    (status) => {
      expect(hasWorkingConversation([{ status }], true)).toBe(false)
    },
  )

  it('ignores old message flags, drafts and armed observers', () => {
    const session = {
      status: 'done',
      draftText: 'unsent',
      messages: [
        { role: 'function-call', running: true, pendingApproval: true },
      ],
    } as Conversation
    expect(hasWorkingConversation([session], true)).toBe(false)
    expect(hasWorkingConversation([], true)).toBe(false)
  })

  it('releases on disconnect instead of trusting stale working status', () => {
    expect(hasWorkingConversation([{ status: 'working' }], false)).toBe(false)
  })
})

describe('waiting on the person’s answer', () => {
  const ask = (output: unknown, running = false) =>
    ({
      id: 'fc_ask',
      role: 'function-call',
      functionId: 'harness::ask',
      input: {},
      output,
      running,
      createdAt: 2,
    }) as Message
  const accepted = ask({
    content: [{ type: 'text', text: 'awaiting answer' }],
    details: { status: 'awaiting_answer', question_id: 'q1' },
  })
  const user = {
    id: 'u1',
    role: 'user',
    content: 'hi',
    createdAt: 1,
  } as Message
  const closing = {
    id: 'a1',
    role: 'assistant',
    content: 'I asked you a question above.',
    createdAt: 3,
  } as Message

  it('a transcript that ends on an accepted ask (even with the model’s closing words) ends on the question', () => {
    expect(endsOnQuestion([user, accepted])).toBe(true)
    expect(endsOnQuestion([user, accepted, closing])).toBe(true)
  })

  it('anything the person said after it, a refused or running ask, or another call does not', () => {
    expect(endsOnQuestion([accepted, { ...user, id: 'u2' }])).toBe(false)
    expect(endsOnQuestion([user, ask({ error: { kind: 'refused' } })])).toBe(
      false,
    )
    expect(endsOnQuestion([user, ask(undefined, true)])).toBe(false)
    expect(
      endsOnQuestion([
        accepted,
        {
          ...(accepted as object),
          id: 'fc2',
          functionId: 'shell::exec',
        } as Message,
      ]),
    ).toBe(false)
    expect(endsOnQuestion([])).toBe(false)
  })

  // The status line read "waiting for Sonnet 5… 2.3s" in the seconds the
  // turn still ran after the card was up.
  it('a working turn whose transcript ends on the question waits on the person', () => {
    const conversation = {
      status: 'working',
      statusReason: 'running tools',
      messages: [user, accepted],
    } as Conversation
    expect(isWaitingOnAnswer(conversation, true)).toBe(true)
  })

  it('so does a finished turn the harness marked awaiting answer, as the sidebar shows', () => {
    const conversation = {
      status: 'done',
      statusReason: 'awaiting answer',
      messages: [user, accepted],
    } as Conversation
    expect(isWaitingOnAnswer(conversation, false)).toBe(true)
  })

  it('a finished turn otherwise does not (stopped, or the person moved on)', () => {
    const conversation = {
      status: 'done',
      messages: [user, accepted],
    } as Conversation
    expect(isWaitingOnAnswer(conversation, false)).toBe(false)
  })
})
