import type { DecisionChatRequest } from '@iii-dev/console-ui'
import type { Selection } from './answer-state'
import type { AskQuestion, NeedsYouItem, TowerQuestion } from '../../src/types'

export type DecisionNeed = Extract<NeedsYouItem, { kind: 'question' | 'held' }>

export function decisionKey(item: DecisionNeed): string {
  return `${item.sessionId}/${item.kind === 'question' ? item.questionId : item.functionCallId}`
}

export function decisionChatRequest(
  item: DecisionNeed,
  record: TowerQuestion | undefined,
  onUseAnswer: DecisionChatRequest['onUseAnswer'],
): DecisionChatRequest {
  const shared = {
    key: decisionKey(item),
    sessionId: item.sessionId,
    kind: item.kind,
    title: `Side chat · ${item.sessionTitle ?? item.title}`,
    sourceTitle: item.sessionTitle,
    createdAt: new Date(item.createdAt).toISOString(),
    card: item.card ? { key: item.card.key, title: item.card.title } : undefined,
    onUseAnswer,
  } as const
  if (item.kind === 'held')
    return {
      ...shared,
      held: {
        functionCallId: item.functionCallId,
        functionId: item.functionId,
        arguments: item.arguments,
      },
    }
  return {
    ...shared,
    agent: item.entry.asked_by,
    why: item.entry.about.why,
    questionId: item.questionId,
    questions: item.entry.questions,
    board: record?.board
      ? {
          ticket: item.card?.key ?? record.ticketId ?? 'the board',
          createdAt: record.board.comment.created_at,
        }
      : undefined,
  }
}

export function selectionFromSideChat(question: AskQuestion, text: string): Selection {
  const answer = text.trim()
  return question.options.some((option) => option.label === answer)
    ? { picked: [answer], text: '' }
    : { picked: [], text: answer }
}
