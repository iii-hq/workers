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
import type { DecisionChatRequest } from '@/types/injectable-ui'

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
  request: DecisionChatRequest,
  assistantText = '',
): SideChatQuote {
  const lines = [
    'This is a conversation about a pending decision. Discuss it without answering or changing the original session.',
  ]
  if (request.kind === 'question') {
    const askerText = assistantText.trim() || request.sourceTitle?.trim()
    if (askerText) lines.push(`What the asker said:\n${askerText}`)
    if (request.why) lines.push(`Why it asked:\n${request.why}`)
    for (const question of request.questions ?? []) {
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
  } else if (request.held) {
    lines.push(
      `Held call:\n${request.held.functionId}\nArguments (redacted): ${
        typeof request.held.arguments === 'string'
          ? request.held.arguments
          : JSON.stringify(request.held.arguments)
      }`,
    )
  }
  if (request.card)
    lines.push(`Card · ${request.card.key}\n${request.card.title}`)
  const trail = [`Asked in ${request.sessionId} at ${request.createdAt}`]
  if (request.board)
    trail.push(`Put on ${request.board.ticket} at ${request.board.createdAt}`)
  lines.push(`Trail:\n${trail.map((line) => `- ${line}`).join('\n')}`)
  return {
    messageId: `decision:${request.key}`,
    text: lines.join('\n\n'),
    ...(request.agent ? { agent: request.agent } : {}),
    full: true,
  }
}

type DecisionClient = Pick<IiiClient, 'call'>
const inFlight = new Map<
  string,
  Promise<{ meta: SessionMeta; quote: SideChatQuote; fresh: boolean }>
>()

export async function findDecisionSideChat(
  iii: DecisionClient,
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
  iii: DecisionClient,
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

async function allowReads(
  iii: DecisionClient,
  sessionId: string,
): Promise<void> {
  await iii.call('approval::set-mode', { session_id: sessionId, mode: 'auto' })
  for (const functionId of SIDE_CHAT_FUNCTIONS) {
    await iii.call('approval::add-always-allow', {
      session_id: sessionId,
      function_id: functionId,
    })
  }
}

function callId(request: DecisionChatRequest): string | undefined {
  return request.kind === 'question'
    ? request.questionId
    : request.held?.functionCallId
}

async function create(
  iii: DecisionClient,
  request: DecisionChatRequest,
): Promise<{ meta: SessionMeta; quote: SideChatQuote; fresh: boolean }> {
  const existing = await findDecisionSideChat(iii, request.key)
  if (existing) {
    await allowReads(iii, existing.session_id)
    return {
      meta: existing,
      quote:
        sideChatQuotes({ sessionMetadata: existing.metadata })[0] ??
        decisionSeed(request),
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
            session_id: request.sessionId,
          })
        )?.meta ?? null
    } catch (error) {
      if (!/session\/not_found/.test(String(error))) throw error
      source = null
    }

    let meta: SessionMeta
    let quote: SideChatQuote
    if (source) {
      const pendingCallId = callId(request)
      const point = pendingCallId
        ? decisionForkPoint(
            await transcript(iii, request.sessionId),
            pendingCallId,
          )
        : null
      if (!point)
        throw new Error('The entry before the pending decision is unavailable.')
      quote = decisionSeed(request, point.assistantText)
      const forked = await iii.call<{ session_id: string; meta: SessionMeta }>(
        'session::fork',
        {
          session_id: request.sessionId,
          entry_id: point.entryId,
          title: request.title,
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
        request.sessionId,
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
      metadata[SIDE_CHAT_QUESTION_KEY] = request.key
      metadata.side_chat_question_kind = request.kind
      metadata[SIDE_CHAT_QUOTES_KEY] = withSideChatQuote([], quote)
      const updated = await iii.call<{ meta: SessionMeta }>(
        'session::set-meta',
        { session_id: createdId, metadata },
      )
      meta = updated.meta
    } else {
      quote = decisionSeed(request)
      const metadata = sideChatMetadata(undefined, request.sessionId)
      metadata[SIDE_CHAT_QUESTION_KEY] = request.key
      metadata.side_chat_question_kind = request.kind
      metadata[SIDE_CHAT_QUOTES_KEY] = withSideChatQuote([], quote)
      delete metadata[SIDE_CHAT_FROM_KEY]
      const made = await iii.call<{ session_id: string; meta: SessionMeta }>(
        'session::create',
        { title: request.title, kind: 'user', metadata },
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
  iii: DecisionClient,
  request: DecisionChatRequest,
): Promise<{ meta: SessionMeta; quote: SideChatQuote; fresh: boolean }> {
  const current = inFlight.get(request.key)
  if (current) return current
  const run = async () => {
    const locks = typeof navigator === 'undefined' ? undefined : navigator.locks
    return locks
      ? locks.request(`iii-decision-side-chat:${request.key}`, () =>
          create(iii, request),
        )
      : create(iii, request)
  }
  const promise = run().finally(() => inFlight.delete(request.key))
  inFlight.set(request.key, promise)
  return promise
}
