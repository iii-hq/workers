import { useEffect, useMemo, useState } from 'react'
import type { ChatBackend } from '@/lib/backend/types'
import { type QuestionCardRef, questionCardsIn } from '@/lib/chat-activity'
import type { Message } from '@/types/chat'

/** A question card still waiting on the person, as the notice lists it. */
export type AwaitingQuestion = Pick<
  QuestionCardRef,
  'messageId' | 'header' | 'title' | 'questionCount'
>

export interface AwaitingQuestionsOptions {
  backend: Pick<ChatBackend, 'askStates' | 'onAskChanged'>
  sessionId: string
  messages: readonly Message[]
  /** The chat waits on the person's answer (`isWaitingOnAnswer`); a change
   *  is a turn boundary, which re-reads the harness. */
  waitingOnAnswer: boolean
}

/**
 * The question cards (`harness::ask`) in this chat that still wait on the
 * person, oldest first, with what each asks (for the notice's list).
 *
 * What the harness knows wins (`askStates`): an open ask entry waits, a
 * settled one does not, whichever tab or channel settled it. A card it
 * knows nothing about (asked through an agent's own tool path, or the read
 * failed) waits until the person says something after it: a reply in the
 * composer answers it as well as the card would, and an old card the chat
 * has long moved past is not news.
 */
export function useAwaitingQuestions({
  backend,
  sessionId,
  messages,
  waitingOnAnswer,
}: AwaitingQuestionsOptions): readonly AwaitingQuestion[] {
  const cards = useMemo(() => questionCardsIn(messages), [messages])
  const lastUserIndex = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index--) {
      if (messages[index].role === 'user') return index
    }
    return -1
  }, [messages])
  const cardKey = cards.map((card) => card.questionId).join('\n')
  const [known, setKnown] = useState<ReadonlyMap<string, boolean> | null>(null)
  const { askStates, onAskChanged } = backend

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new card or a turn boundary is the re-read trigger.
  useEffect(() => {
    if (!askStates || cardKey === '') {
      setKnown(null)
      return
    }
    let live = true
    let request = 0
    const refresh = () => {
      const mine = ++request
      askStates(sessionId).then(
        (states) => {
          if (live && mine === request) setKnown(states)
        },
        () => {
          if (live && mine === request) setKnown(null)
        },
      )
    }
    refresh()
    const off = onAskChanged?.(sessionId, refresh)
    return () => {
      live = false
      off?.()
    }
  }, [askStates, onAskChanged, sessionId, cardKey, waitingOnAnswer])

  const waiting = useMemo(
    () =>
      cards.filter(
        (card) => known?.get(card.questionId) ?? card.index > lastUserIndex,
      ),
    [cards, known, lastUserIndex],
  )
  // One array per distinct answer, so the list's effects only re-run when
  // the set of waiting cards (or what they say) changes.
  const key = waiting
    .map((card) => `${card.messageId}\u0000${card.header}\u0000${card.title}`)
    .join('\n')
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed by content.
  return useMemo(
    () =>
      waiting.map(({ messageId, header, title, questionCount }) => ({
        messageId,
        header,
        title,
        questionCount,
      })),
    [key],
  )
}
