/**
 * The engine's side chats (chat-projects §2.2, D79–D85): `harness::side-chat::
 * open`, `close`, `keep` and `summarize`. All four are internal control-plane
 * functions (D37); the ADE uses them when the catalogue has `open`
 * (`ChatCapabilities.sideChats`) and keeps its own client-side fork otherwise.
 *
 * Refusals arrive as one JSON object after `handler error: ` (D33):
 * `nested_side_chat`, `nothing_to_fork`, `not_found`, `not_side_chat`,
 * `side_chat_busy` and, from `harness::send`, `side_chat_locked`.
 */
import { errText } from '@/lib/errors'
import { getIiiClient } from '@/lib/iii-client'
import type { SessionMeta } from '@/lib/sessions/types'
import { parseProjectError } from './projects'

export const SIDE_CHAT_FUNCTIONS = {
  open: 'harness::side-chat::open',
  close: 'harness::side-chat::close',
  keep: 'harness::side-chat::keep',
  summarize: 'harness::side-chat::summarize',
} as const

/** A passage of the conversation the side chat opens about. */
export interface SideChatOpenQuote {
  messageId: string
  text: string
  agent?: string
}

export interface SideChatOpenInput {
  /** The conversation to fork. */
  sessionId: string
  /** The conversation's entry to fork at; the engine's default is its last
   *  user or assistant message. */
  entryId?: string
  quote?: SideChatOpenQuote
}

export interface SideChatOpened {
  session_id: string
  meta: SessionMeta
}

export async function openSideChat(
  input: SideChatOpenInput,
): Promise<SideChatOpened> {
  const client = await getIiiClient()
  return client.call<SideChatOpened>(SIDE_CHAT_FUNCTIONS.open, {
    session_id: input.sessionId,
    ...(input.entryId ? { entry_id: input.entryId } : {}),
    ...(input.quote
      ? {
          quote: {
            message_id: input.quote.messageId,
            text: input.quote.text,
            ...(input.quote.agent ? { agent: input.quote.agent } : {}),
          },
        }
      : {}),
  })
}

/** Deletes the side chat when nothing was sent in it (D83); otherwise it
 *  stays and `deleted` is false. */
export async function closeSideChat(
  sessionId: string,
): Promise<{ deleted: boolean; operation_id?: string }> {
  const client = await getIiiClient()
  return client.call(SIDE_CHAT_FUNCTIONS.close, { session_id: sessionId })
}

/** An ordinary conversation from now on, in its conversation's project. */
export async function keepSideChat(
  sessionId: string,
): Promise<{ meta: SessionMeta }> {
  const client = await getIiiClient()
  return client.call(SIDE_CHAT_FUNCTIONS.keep, { session_id: sessionId })
}

/** Sends the fixed summary prompt in the side chat (`side_chat_summary`). */
export async function summarizeSideChat(
  sessionId: string,
): Promise<{ turn_id: string }> {
  const client = await getIiiClient()
  return client.call(SIDE_CHAT_FUNCTIONS.summarize, { session_id: sessionId })
}

/** The engine's refusal code, when the error carries one. */
export function sideChatErrorCode(err: unknown): string | null {
  return parseProjectError(err)?.code ?? null
}

const ERROR_COPY: Record<string, string> = {
  nested_side_chat: 'This is already a side chat. Ask in it directly.',
  nothing_to_fork:
    'A side chat starts from a reply, and this conversation has none yet. Send a message first.',
  not_found: 'This side chat no longer exists.',
  not_side_chat: 'This conversation is no longer a side chat.',
  side_chat_busy:
    'The side chat is still answering. Ask for the summary once it is done.',
  side_chat_locked:
    "A side chat keeps its conversation's model and effort, to reuse its cache.",
}

/** A sentence for a side chat refusal, else the error's own text. */
export function describeSideChatError(err: unknown): string {
  const parsed = parseProjectError(err)
  if (parsed && ERROR_COPY[parsed.code]) return ERROR_COPY[parsed.code]
  return parsed?.message ?? errText(err)
}
