/**
 * Keep flatMessagesKey in sync with turn-orchestrator/state.ts::messagesKey.
 * Importing it directly would create a package-layer cycle (orchestrator
 * depends on context-compaction via preflight). A drift-guard test asserts
 * the two stay identical.
 */

import type { ISdk } from '../runtime/iii.js';
import { stateSet } from '../runtime/state.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';

const FLAT_STATE_SCOPE = 'agent';

export function flatMessagesKey(session_id: string): string {
  return `session/${session_id}/messages`;
}

export function buildSummaryMessage(summary_text: string): AssistantMessage {
  return {
    role: 'assistant',
    content: [
      {
        type: 'text',
        text: `<conversation-summary>\n${summary_text}\n</conversation-summary>`,
      },
    ],
    stop_reason: 'end',
    error_message: null,
    error_kind: null,
    usage: null,
    model: '',
    provider: '',
    timestamp: Date.now(),
  };
}

export async function rewriteFlatMessages(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
): Promise<void> {
  await stateSet(iii, FLAT_STATE_SCOPE, flatMessagesKey(session_id), messages);
}
