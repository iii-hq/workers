import { describe, expect, it, vi } from 'vitest'
import { towerFixture } from './fixtures'
import type { AskQuestion, HeldNeed, QuestionNeed } from '../../src/types'
import { decisionChatRequest, decisionKey, selectionFromSideChat } from './side-chat'

const question = towerFixture.items.find((item) => item.kind === 'question') as QuestionNeed
const held = towerFixture.items.find((item) => item.kind === 'held') as HeldNeed

describe('decision side chat page mapping', () => {
  it('maps a question to the host contract', () => {
    const onUseAnswer = vi.fn()
    const record = towerFixture.questions.find((row) => row.questionId === question.questionId)
    expect(decisionChatRequest(question, record, onUseAnswer)).toEqual({
      key: decisionKey(question),
      sessionId: question.sessionId,
      kind: 'question',
      title: `Side chat · ${question.sessionTitle ?? question.title}`,
      sourceTitle: question.sessionTitle,
      createdAt: new Date(question.createdAt).toISOString(),
      card: question.card ? { key: question.card.key, title: question.card.title } : undefined,
      agent: question.entry.asked_by,
      why: question.entry.about.why,
      questionId: question.questionId,
      questions: question.entry.questions,
      board: record?.board
        ? {
            ticket: question.card?.key ?? record.ticketId ?? 'the board',
            createdAt: record.board.comment.created_at,
          }
        : undefined,
      onUseAnswer,
    })
  })

  it('maps a held call and its already-redacted arguments', () => {
    expect(decisionChatRequest(held, undefined, vi.fn())).toMatchObject({
      key: decisionKey(held),
      sessionId: held.sessionId,
      kind: 'held',
      held: {
        functionCallId: held.functionCallId,
        functionId: held.functionId,
        arguments: held.arguments,
      },
    })
  })

  it('maps an exact option label or arbitrary text into the decision draft', () => {
    const single = question.entry.questions[0]
    expect(selectionFromSideChat(single, `  ${single.options[1].label}  `)).toEqual({
      picked: [single.options[1].label],
      text: '',
    })
    expect(selectionFromSideChat(single, 'A different answer')).toEqual({
      picked: [],
      text: 'A different answer',
    })
    const multi: AskQuestion = { ...single, multi_select: true }
    expect(selectionFromSideChat(multi, multi.options[0].label)).toEqual({
      picked: [multi.options[0].label],
      text: '',
    })
  })
})
