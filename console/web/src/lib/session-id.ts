/**
 * Message ID helper.
 *
 * One fresh `msg-<uuid>` per send. It flows into the engine via baggage so
 * the traces UI can "Group by message", and the harness derives the user
 * message's session-manager entry id from it (`<message_id>-user-0`) — which
 * is also how the console's optimistic user row reconciles in place.
 *
 * (Conversation ids are minted with the `console-` prefix directly in
 * use-conversations and double as the engine session_id, so there is no
 * separate session-id mapper anymore.)
 */

const MESSAGE_PREFIX = 'msg-'

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
