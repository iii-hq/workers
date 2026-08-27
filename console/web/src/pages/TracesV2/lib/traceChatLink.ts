// Chat linkage of one trace: which conversation (and which turn of it)
// produced the spans. Every span a harness turn spawns carries the identity
// as baggage attributes (`harness/src/functions/turn.rs`), and
// `engine::traces::list` merges them into the row's trace-level tags — so a
// list row resolves from `trace_tags` alone, while a detail opened without
// its row (timeline-strip click, deep link) falls back to scanning the
// loaded spans' own attributes.

import { SESSION_ID_ATTR } from './followTurn'

/** `iii.message.id` — the harness TURN id (`t_<uuid>`), not the console's
 *  `msg-<uuid>`. See timeline-span-tags.md. */
export const MESSAGE_ID_ATTR = 'iii.message.id'
export const SESSION_NAME_ATTR = 'iii.session.name'

export interface TraceChatLink {
  /** engine session_id (= the console's `Conversation.id`) */
  sessionId: string
  /** session title at emit time, for the button label */
  sessionName?: string
  /** harness turn id, when the trace belongs to a single turn */
  turnId?: string
}

function str(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

/**
 * Resolve a trace's chat linkage from its merged trace tags, falling back
 * to the first span attributes that carry each key. Returns null when no
 * span of the trace was stamped with a session id (engine-internal traces,
 * non-harness workloads).
 */
export function traceChatLink(
  traceTags: Record<string, string> | undefined,
  spans: ReadonlyArray<{ attributes?: Record<string, unknown> }>,
): TraceChatLink | null {
  let sessionId = str(traceTags?.[SESSION_ID_ATTR])
  let sessionName = str(traceTags?.[SESSION_NAME_ATTR])
  let turnId = str(traceTags?.[MESSAGE_ID_ATTR])
  for (const span of spans) {
    if (sessionId && sessionName && turnId) break
    const attrs = span.attributes
    if (!attrs) continue
    sessionId ??= str(attrs[SESSION_ID_ATTR])
    sessionName ??= str(attrs[SESSION_NAME_ATTR])
    turnId ??= str(attrs[MESSAGE_ID_ATTR])
  }
  if (!sessionId) return null
  return { sessionId, sessionName, turnId }
}
