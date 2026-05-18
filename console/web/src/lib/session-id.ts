/**
 * Session/message ID helpers.
 *
 * Mirrors `workers/harness/web/src/App.tsx` `newSessionId()` and the
 * per-send `msg-${crypto.randomUUID()}` pattern: one stable session_id
 * per conversation, one fresh message_id per send. Both flow into the
 * engine via baggage so the traces UI can "Group by session" and
 * "Group by message" without exploding.
 *
 * We prefix conversation IDs with `console-` so the trace UI can tell
 * console-emitted sessions from harness CLI / cron / other producers
 * at a glance — same convention as `workers/harness/web` (which
 * prefixes its sessions with the `s` date stamp).
 */

const SESSION_PREFIX = 'console-'
const MESSAGE_PREFIX = 'msg-'

/**
 * Derive the engine `session_id` from a conversation's stable id.
 * Idempotent: if the id is already prefixed, the prefix is not added
 * twice. Stored conversations whose id was minted before this helper
 * existed get the prefix tacked on at the boundary; the localStorage
 * value itself is left alone.
 */
export function makeSessionId(conversationId: string): string {
  if (conversationId.startsWith(SESSION_PREFIX)) return conversationId
  return `${SESSION_PREFIX}${conversationId}`
}

/**
 * Mint a fresh message_id for a single turn. Called once per `send()`
 * so every user turn lands under its own `iii.message.id` bucket in
 * the traces UI's "Group by message" view.
 */
export function newMessageId(): string {
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `${MESSAGE_PREFIX}${crypto.randomUUID()}`
  }
  // Fallback for environments without crypto.randomUUID (older WebViews).
  const rand = Math.random().toString(36).slice(2, 10)
  return `${MESSAGE_PREFIX}${Date.now().toString(36)}-${rand}`
}
