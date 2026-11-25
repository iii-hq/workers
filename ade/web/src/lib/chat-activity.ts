import type { Conversation, Message } from '@/types/chat'

/**
 * Session status covers model wait/reasoning/answering, tools and approvals,
 * including subagents and sessions in other workspace panels. Never infer
 * liveness from old transcript flags, armed observers or a connected socket.
 */
export function hasWorkingConversation(
  conversations: readonly Pick<Conversation, 'status'>[],
  connected: boolean,
): boolean {
  return (
    connected && conversations.some((session) => session.status === 'working')
  )
}

/**
 * The `done` reason the harness projects when a turn ended on a
 * `harness::ask` card (`harness/src/session_status.rs`): the agent is
 * waiting on the user's answer, which arrives as their next message.
 */
export const AWAITING_ANSWER_REASON = 'awaiting answer'

/** Whether the session's last turn ended on a question the user has not answered. */
export function isAwaitingAnswer(
  conversation: Pick<Conversation, 'status' | 'statusReason'>,
): boolean {
  return (
    conversation.status === 'done' &&
    conversation.statusReason === AWAITING_ANSWER_REASON
  )
}

/** The function id of the agent's question card. */
const ASK_FUNCTION_ID = 'harness::ask'
/** `details.status` of an ask the harness accepted and showed. */
const ASK_AWAITING_STATUS = 'awaiting_answer'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The details of an accepted `harness::ask` result (`{ content, details:
 *  { status, question_id, … } }`), or `null`. */
function acceptedAskDetails(message: Message): Record<string, unknown> | null {
  if (message.role !== 'function-call') return null
  if (message.functionId !== ASK_FUNCTION_ID || message.running) return null
  const { output } = message
  if (!isRecord(output) || 'error' in output) return null
  const details = Array.isArray(output.content) ? output.details : output
  return isRecord(details) && details.status === ASK_AWAITING_STATUS
    ? details
    : null
}

function isAcceptedAsk(message: Message): boolean {
  return acceptedAskDetails(message) !== null
}

/** One question card in the transcript: its row and the harness's id for it. */
export interface QuestionCardRef {
  /** The function-call message the card renders in. */
  messageId: string
  /** `harness::ask` question id (`harness::ask::list` → `question_id`). */
  questionId: string
  /** Its position in the messages it was read from. */
  index: number
  /** The first question's header (`Colour`), when it has one. */
  header?: string
  /** The first question's text, or the call's description. */
  title: string
  /** How many questions the card asks. */
  questionCount: number
}

function firstQuestion(details: Record<string, unknown>) {
  const questions = Array.isArray(details.questions) ? details.questions : []
  const first = questions[0]
  return {
    count: questions.length,
    header:
      isRecord(first) && typeof first.header === 'string' && first.header
        ? first.header
        : undefined,
    question:
      isRecord(first) && typeof first.question === 'string' && first.question
        ? first.question
        : undefined,
  }
}

/** Every question card the agent showed in `messages`, oldest first. */
export function questionCardsIn(
  messages: readonly Message[],
): QuestionCardRef[] {
  const cards: QuestionCardRef[] = []
  messages.forEach((message, index) => {
    const details = acceptedAskDetails(message)
    if (details && typeof details.question_id === 'string') {
      const first = firstQuestion(details)
      const description =
        message.role === 'function-call' ? message.description : undefined
      cards.push({
        messageId: message.id,
        questionId: details.question_id,
        index,
        header: first.header,
        title: first.question ?? description ?? 'Question from the agent',
        questionCount: Math.max(1, first.count),
      })
    }
  })
  return cards
}

/**
 * Whether the transcript ends on the agent's question: the latest call is an
 * accepted `harness::ask` and nothing the person (or an observer) said came
 * after it. Only the model's closing words and reasoning may follow. The
 * card is up from here on, while the turn may still be winding down (an
 * agent such as claude-code finishes its own step after the ask), before
 * the harness projects `awaiting answer`.
 */
export function endsOnQuestion(messages: readonly Message[]): boolean {
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index]
    if (message.role === 'assistant' || message.role === 'thought') continue
    if (message.role === 'system' && message.kind !== 'observer-fired') continue
    return isAcceptedAsk(message)
  }
  return false
}

/** What the chat says while it waits on the person's answer (the sidebar
 *  row's words). */
export const WAITING_ON_ANSWER_LABEL = 'waiting for your answer'

/**
 * Whether the chat waits on the person's answer to a question card: the
 * harness says so (`isAwaitingAnswer`, as the sidebar shows it), or a
 * working turn already ends on the question it asked. Either way the chat
 * should say it waits for the person, not for a model.
 */
export function isWaitingOnAnswer(
  conversation: Pick<Conversation, 'status' | 'statusReason' | 'messages'>,
  working: boolean,
): boolean {
  return (
    isAwaitingAnswer(conversation) ||
    (working && endsOnQuestion(conversation.messages))
  )
}
