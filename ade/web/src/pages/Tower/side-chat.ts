import {
  SIDE_CHAT_FROM_KEY,
  SIDE_CHAT_KEEP_KEY,
  SIDE_CHAT_QUESTION_KEY,
  SIDE_CHAT_QUOTES_KEY,
  type SideChatQuote,
  sideChatMetadata,
  sideChatQuote,
  sideChatQuotes,
  withSideChatQuote,
} from '@/lib/chat-aside'
import type { IiiClient } from '@/lib/iii-client'
import type { SessionMeta, TranscriptItem } from '@/lib/sessions/types'
import type { Selection } from './answer-state'
import type { AskQuestion, NeedsYouItem, TowerQuestion } from './model'

export { SIDE_CHAT_KEEP_KEY, SIDE_CHAT_QUESTION_KEY }
export const SIDE_CHAT_FUNCTIONS = [
  'session::get',
  'session::messages-tail',
  'harness::status',
  'kanban::ticket::get',
  'kanban::ticket::list',
  'kanban::question::list',
  'coder::read-file',
  'memory::recall',
] as const

export type DecisionNeed = Extract<NeedsYouItem, { kind: 'question' | 'held' }>

export function decisionKey(item: DecisionNeed): string {
  return `${item.sessionId}/${
    item.kind === 'question' ? item.questionId : item.functionCallId
  }`
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function isCall(block: unknown, callId: string): boolean {
  const call = record(block)
  if (call?.type !== 'function_call') return false
  if (call.id === callId) return true
  if (
    call.function_id !== 'agent_call' &&
    call.function_id !== 'harness::agent_call'
  )
    return false
  const args = record(call.arguments)
  const payload = record(args?.payload)
  return (
    args?.id === callId ||
    args?.function_call_id === callId ||
    payload?.id === callId ||
    payload?.function_call_id === callId
  )
}

/** The entry immediately before the assistant step that made the pending call. */
export function decisionForkPoint(
  items: readonly TranscriptItem[],
  callId: string,
): { entryId: string; assistantText: string } | null {
  for (let index = 0; index < items.length; index += 1) {
    const message = items[index].message
    if (
      message?.role !== 'assistant' ||
      !message.content.some((block) => isCall(block, callId))
    )
      continue
    if (index === 0) return null
    return {
      entryId: items[index - 1].entry_id,
      assistantText: message.content
        .filter(
          (block): block is Extract<typeof block, { type: 'text' }> =>
            block.type === 'text',
        )
        .map((block) => block.text.trim())
        .filter(Boolean)
        .join('\n\n'),
    }
  }
  return null
}

export function decisionSeed(
  item: DecisionNeed,
  assistantText = '',
  questionRecord?: TowerQuestion,
): SideChatQuote {
  const lines = [
    'This is a conversation about a pending decision. Discuss it without answering or changing the original session.',
  ]
  if (assistantText.trim())
    lines.push(`What the asker said:\n${assistantText.trim()}`)
  if (item.kind === 'question') {
    if (item.entry.about.why)
      lines.push(`Why it asked:\n${item.entry.about.why}`)
    for (const question of item.entry.questions) {
      lines.push(`Question · ${question.header}\n${question.question}`)
      lines.push(
        `Options:\n${question.options
          .map(
            (option) =>
              `- ${option.label}${option.recommended ? ' (recommended)' : ''}${
                option.description ? ` — ${option.description}` : ''
              }`,
          )
          .join('\n')}`,
      )
    }
  } else {
    lines.push(
      `Held call:\n${item.functionId}\nArguments (redacted): ${
        typeof item.arguments === 'string'
          ? item.arguments
          : JSON.stringify(item.arguments)
      }`,
    )
  }
  if (item.card) lines.push(`Card · ${item.card.key}\n${item.card.title}`)
  const trail = [
    `Asked in ${item.sessionId} at ${new Date(item.createdAt).toISOString()}`,
  ]
  if (questionRecord?.board)
    trail.push(
      `Put on ${item.card?.key ?? questionRecord.ticketId ?? 'the board'} at ${questionRecord.board.comment.created_at}`,
    )
  lines.push(`Trail:\n${trail.map((line) => `- ${line}`).join('\n')}`)
  return {
    messageId: `decision:${decisionKey(item)}`,
    text: lines.join('\n\n'),
    ...(item.kind === 'question' && item.entry.asked_by
      ? { agent: item.entry.asked_by }
      : {}),
    full: true,
  }
}

export function selectionFromSideChat(
  question: AskQuestion,
  text: string,
): Selection {
  const answer = text.trim()
  return question.options.some((option) => option.label === answer)
    ? { picked: [answer], text: '' }
    : { picked: [], text: answer }
}

type TowerClient = Pick<IiiClient, 'call'>
const inFlight = new Map<
  string,
  Promise<{ meta: SessionMeta; quote: SideChatQuote; fresh: boolean }>
>()

async function listCopy(
  iii: TowerClient,
  key: string,
): Promise<SessionMeta | null> {
  let cursor: string | undefined
  do {
    const page = await iii.call<{
      sessions?: SessionMeta[]
      next_cursor?: string | null
    }>('session::list', {
      limit: 200,
      metadata: { [SIDE_CHAT_QUESTION_KEY]: key },
      ...(cursor ? { cursor } : {}),
    })
    const copy = page.sessions?.find(
      (session) => session.metadata?.[SIDE_CHAT_QUESTION_KEY] === key,
    )
    if (copy) return copy
    cursor = page.next_cursor ?? undefined
  } while (cursor)
  return null
}

async function transcript(
  iii: TowerClient,
  sessionId: string,
): Promise<TranscriptItem[]> {
  const items: TranscriptItem[] = []
  let cursor: string | undefined
  do {
    const page = await iii.call<{
      messages?: TranscriptItem[]
      next_cursor?: string | null
    }>('session::messages', {
      session_id: sessionId,
      limit: 500,
      include_custom: true,
      include_image_data: false,
      ...(cursor ? { cursor } : {}),
    })
    items.push(...(page.messages ?? []))
    cursor = page.next_cursor ?? undefined
  } while (cursor)
  return items
}

async function allowReads(iii: TowerClient, sessionId: string): Promise<void> {
  await iii.call('approval::set-mode', { session_id: sessionId, mode: 'auto' })
  for (const functionId of SIDE_CHAT_FUNCTIONS) {
    await iii.call('approval::add-always-allow', {
      session_id: sessionId,
      function_id: functionId,
    })
  }
}

async function create(
  iii: TowerClient,
  item: DecisionNeed,
  questionRecord?: TowerQuestion,
): Promise<{ meta: SessionMeta; quote: SideChatQuote; fresh: boolean }> {
  const key = decisionKey(item)
  const existing = await listCopy(iii, key)
  if (existing) {
    await allowReads(iii, existing.session_id)
    return {
      meta: existing,
      quote:
        sideChatQuotes({ sessionMetadata: existing.metadata })[0] ??
        decisionSeed(item),
      fresh: false,
    }
  }

  let createdId: string | null = null
  try {
    let source: SessionMeta | null
    try {
      source =
        (
          await iii.call<{ meta: SessionMeta } | null>('session::get', {
            session_id: item.sessionId,
          })
        )?.meta ?? null
    } catch (error) {
      if (!/session\/not_found/.test(String(error))) throw error
      source = null
    }

    let meta: SessionMeta
    let quote: SideChatQuote
    if (source) {
      const point = decisionForkPoint(
        await transcript(iii, item.sessionId),
        item.kind === 'question' ? item.questionId : item.functionCallId,
      )
      if (!point)
        throw new Error('The entry before the pending decision is unavailable.')
      quote = decisionSeed(item, point.assistantText, questionRecord)
      const forked = await iii.call<{ session_id: string; meta: SessionMeta }>(
        'session::fork',
        {
          session_id: item.sessionId,
          entry_id: point.entryId,
          title: `Side chat · ${source.title || item.title}`,
        },
      )
      createdId = forked.session_id
      const copied = await iii.call<{ messages?: TranscriptItem[] }>(
        'session::messages-tail',
        {
          session_id: createdId,
          limit: 1,
          include_custom: true,
          include_image_data: false,
        },
      )
      const metadata = sideChatMetadata(
        forked.meta.metadata,
        item.sessionId,
        copied.messages?.at(-1)?.entry_id,
      )
      for (const key of [
        'kanban_ticket',
        'external_source',
        'read_only',
        'agent_profile',
        'system_prompt',
        'skills',
        SIDE_CHAT_KEEP_KEY,
      ])
        delete metadata[key]
      metadata[SIDE_CHAT_QUESTION_KEY] = key
      metadata.side_chat_question_kind = item.kind
      metadata[SIDE_CHAT_QUOTES_KEY] = withSideChatQuote([], quote)
      const updated = await iii.call<{ meta: SessionMeta }>(
        'session::set-meta',
        {
          session_id: createdId,
          metadata,
        },
      )
      meta = updated.meta
    } else {
      quote = decisionSeed(item, '', questionRecord)
      const metadata = sideChatMetadata(undefined, item.sessionId)
      metadata[SIDE_CHAT_QUESTION_KEY] = key
      metadata.side_chat_question_kind = item.kind
      metadata[SIDE_CHAT_QUOTES_KEY] = withSideChatQuote([], quote)
      delete metadata[SIDE_CHAT_FROM_KEY]
      const made = await iii.call<{ session_id: string; meta: SessionMeta }>(
        'session::create',
        {
          title: `Side chat · ${item.sessionTitle || item.title}`,
          kind: 'user',
          metadata,
        },
      )
      createdId = made.session_id
      meta = made.meta
    }
    await allowReads(iii, createdId)
    await iii.call('session::append', {
      session_id: createdId,
      message: {
        role: 'user',
        content: [{ type: 'text', text: sideChatQuote(quote.text, true) }],
        timestamp: Date.now(),
      },
    })
    return { meta, quote, fresh: true }
  } catch (error) {
    if (createdId)
      await iii
        .call('session::delete', { session_id: createdId })
        .catch(() => undefined)
    throw error
  }
}

/** Create or recover the one isolated read-only side chat for a decision. */
export function createDecisionSideChat(
  iii: TowerClient,
  item: DecisionNeed,
  questionRecord?: TowerQuestion,
): Promise<{ meta: SessionMeta; quote: SideChatQuote; fresh: boolean }> {
  const key = decisionKey(item)
  const current = inFlight.get(key)
  if (current) return current
  const run = async () => {
    const locks = typeof navigator === 'undefined' ? undefined : navigator.locks
    return locks
      ? locks.request(`iii-decision-side-chat:${key}`, () =>
          create(iii, item, questionRecord),
        )
      : create(iii, item, questionRecord)
  }
  const promise = run().finally(() => inFlight.delete(key))
  inFlight.set(key, promise)
  return promise
}
