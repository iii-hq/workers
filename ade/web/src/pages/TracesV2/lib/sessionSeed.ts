/**
 * Which trace Traces opens with when it comes up beside a chat (the "session
 * seed"): the newest turn of the chat that is RUNNING when one can be told
 * apart, else of the active chat.
 *
 * Running comes from the conversation record's `status`, which the console
 * sets to `working` when a turn starts and `session::status-changed` keeps
 * current. It picks a chat only when it is unambiguous: the active chat when
 * it is the one running, or the one chat running when the active one is
 * idle. Several chats running with the active one idle has no single answer,
 * and the active chat stands — the list follows it either way.
 */

import type { StoredSpan } from '../api/traces'
import type { TraceListItem } from '../hooks/useTraceData'
import { turnTracesFor } from './followTurn'
import { traceChatLink } from './traceChatLink'

export interface SeedConversation {
  id: string
  status?: string
  /** A sub-agent's session: its turns are not a chat the operator runs. */
  parentId?: string
  draft?: boolean
}

/** The sessions to seed from, best first. */
export function seedSessions(
  activeId: string | null,
  conversations: readonly SeedConversation[],
): string[] {
  const running = conversations
    .filter((c) => c.status === 'working' && !c.parentId && !c.draft)
    .map((c) => c.id)
  const order: string[] = []
  if (activeId && running.includes(activeId)) order.push(activeId)
  else if (running.length === 1) order.push(running[0])
  if (activeId && !order.includes(activeId)) order.push(activeId)
  return order
}

/**
 * The trace to open for the first of `sessions` that has one: its newest
 * turn on the live feed, else — the feed reaches back two minutes — its
 * newest row in the list, matched by the row's session tag (so a row of the
 * unscoped list that lands before the scoped one never qualifies). Rows
 * arrive newest first.
 */
export function seedTrace(
  sessions: readonly string[],
  spans: readonly StoredSpan[],
  rows: readonly Pick<TraceListItem, 'traceId' | 'traceTags'>[],
): string | null {
  for (const sessionId of sessions) {
    const turns = turnTracesFor(spans, sessionId)
    if (turns.size > 0) {
      return [...turns].reduce((a, b) => (b[1] > a[1] ? b : a))[0]
    }
    const row = rows.find(
      (trace) => traceChatLink(trace.traceTags, [])?.sessionId === sessionId,
    )
    if (row) return row.traceId
  }
  return null
}
