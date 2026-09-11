/**
 * Message / session ID helpers.
 *
 * One fresh `msg-<uuid>` per send. It rides `harness::send` as the
 * `idempotency_key`, from which the harness derives the user message's
 * session-manager entry id (`e_idem_<message_id>`) — which is also how the
 * console's optimistic user row reconciles in place (see
 * `predictedUserEntryId` in lib/backend/harness-send.ts) — and travels in
 * `options.metadata.message_id`. It does NOT reach the trace baggage: the
 * `iii.message.id` span attribute the traces UI groups by is the harness
 * TURN id (`t_<uuid>`, see harness/src/functions/turn.rs), correlated back
 * to transcript rows via the `e_<turn_id>_…` entry ids (lib/turn-anchor.ts).
 *
 * (Conversation ids are minted with the `console-` prefix via
 * [`newSessionId`] and double as the engine session_id, so there is no
 * separate session-id mapper anymore.)
 */

const MESSAGE_PREFIX = 'msg-'
const SESSION_PREFIX = 'console-'

/**
 * `crypto.randomUUID` only exists in secure contexts (https / localhost) —
 * a console served over `http://<LAN-IP>` has `crypto` without it — so
 * fall back to a timestamp+random id rather than throwing.
 */
function mintId(prefix: string): string {
  if (
    typeof crypto !== 'undefined' &&
    typeof crypto.randomUUID === 'function'
  ) {
    return `${prefix}${crypto.randomUUID()}`
  }
  const rand = Math.random().toString(36).slice(2, 10)
  return `${prefix}${Date.now().toString(36)}-${rand}`
}

/**
 * Mint a fresh message_id for a single turn. Called once per `send()` so
 * every user turn gets its own idempotency key (and thus its own
 * `e_idem_<message_id>` transcript entry).
 */
export function newMessageId(): string {
  return mintId(MESSAGE_PREFIX)
}

/**
 * Mint a fresh `console-<uuid>` session/conversation id (doubles as the
 * engine session_id).
 */
export function newSessionId(): string {
  return mintId(SESSION_PREFIX)
}
