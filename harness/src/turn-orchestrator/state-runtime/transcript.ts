/**
 * Shared transcript idempotency helpers for turn FSM handlers.
 */

import type { AgentMessage, AssistantMessage } from '../../types/agent-message.js';

/**
 * Function_call_ids already persisted for the current turn. Results are appended
 * right after the assistant that requested them, so they form the trailing run
 * of `function_result` messages; the first non-result from the tail is the turn
 * boundary.
 */
export function persistedTrailingResultIds(messages: AgentMessage[]): Set<string> {
  const ids = new Set<string>();
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role === 'function_result') ids.add(m.function_call_id);
    else break;
  }
  return ids;
}

/** True when the trailing assistant message matches the candidate (re-entry dup). */
export function isDuplicateAssistant(
  messages: AgentMessage[],
  asst: AssistantMessage,
): boolean {
  const last = messages[messages.length - 1];
  return (
    last !== undefined &&
    last.role === 'assistant' &&
    last.timestamp === asst.timestamp &&
    last.model === asst.model &&
    last.provider === asst.provider
  );
}
