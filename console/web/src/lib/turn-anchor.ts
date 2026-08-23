import type { Message } from '@/types/chat'

/**
 * The transcript row a trace's "go to message" should land on, for one
 * harness turn.
 *
 * Harness-derived entry ids embed the turn id — `e_<turn_id>_<step>_assistant`,
 * `e_<turn_id>_<function_call_id>`, `e_<turn_id>_stopped` (harness/src/ids.rs)
 * — so the turn's rows are found by prefix. The USER row that started the
 * turn (`e_idem_<message_id>` / `e_q_<id>`) does not carry the turn id; it is
 * recovered positionally as the nearest user row directly above the turn's
 * first entry, which reads better as a landing point than an arbitrary tool
 * call. The backward walk skips local-only rows (uid-based system notices)
 * but stops at any other durable entry — a user row beyond one belongs to a
 * different exchange.
 *
 * Returns null when the transcript holds no entry of that turn (e.g. a turn
 * still streaming on optimistic local ids, or a pruned history).
 */
export function turnAnchorMessageId(
  messages: ReadonlyArray<Message>,
  turnId: string,
): string | null {
  if (!turnId) return null
  const prefix = `e_${turnId}_`
  const first = messages.findIndex((m) => m.id.startsWith(prefix))
  if (first === -1) return null
  for (let i = first - 1; i >= 0; i--) {
    const m = messages[i]
    if (m.role === 'user') return m.id
    if (m.id.startsWith('e_')) break
  }
  return messages[first].id
}
