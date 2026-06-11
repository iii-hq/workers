/**
 * Shared transcript idempotency helpers for turn FSM handlers.
 */

import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';

function assistantHasFunctionCalls(m: AssistantMessage): boolean {
  return m.content.some((b) => b.type === 'function_call');
}

/**
 * Function_call_ids already persisted for the current turn. Results are appended
 * right after the assistant that requested them, so they form the trailing run
 * of `function_result` messages; the requesting assistant (which carries the
 * function calls) is the turn boundary.
 *
 * A trailing assistant with NO function calls is skipped, not treated as the
 * boundary: on a max_turns crash-replay the synthetic "loop stopped" notice is
 * appended after the results in the same step, and the dedup must still see the
 * results behind it as persisted — otherwise the whole batch re-appends.
 */
export function persistedTrailingResultIds(messages: AgentMessage[]): Set<string> {
  const ids = new Set<string>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (!m) break;
    if (m.role === 'function_result') {
      ids.add(m.function_call_id);
      continue;
    }
    if (m.role === 'assistant' && !assistantHasFunctionCalls(m)) continue;
    break;
  }
  return ids;
}

/** True when the trailing assistant message matches the candidate (re-entry dup). */
export function isDuplicateAssistant(messages: AgentMessage[], asst: AssistantMessage): boolean {
  const last = messages[messages.length - 1];
  return (
    last !== undefined &&
    last.role === 'assistant' &&
    last.timestamp === asst.timestamp &&
    last.model === asst.model &&
    last.provider === asst.provider
  );
}
