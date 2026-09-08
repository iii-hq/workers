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

/**
 * The one entry every harness turn that generated anything wrote first: its
 * first generate step, `e_<turn_id>_0_assistant`. Steps count from 0: the
 * harness creates a turn record at `step: 0` and names the assistant entry
 * after the step that produced it (harness/src/functions/send.rs,
 * harness/src/turn_loop.rs `ids::assistant_entry_id(turn_id, payload.step)`).
 * A paged transcript uses it as the `until_entry_id` anchor to widen the
 * loaded window back to a turn the reader was linked to but which is not
 * loaded yet. The user row above it may still be on the page before;
 * `turnAnchorMessageId` then lands on this entry instead.
 */
export function turnFirstEntryId(turnId: string): string {
  return `e_${turnId}_0_assistant`
}
