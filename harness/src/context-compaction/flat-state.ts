/**
 * Rewrite flat transcript messages in scope `messages`.
 */

import type { ISdk } from '../runtime/iii.js';
import { stateSet } from '../runtime/state.js';
import { MESSAGES_SCOPE } from '../turn-orchestrator/state.js';
import type { AgentMessage, AssistantMessage } from '../types/agent-message.js';

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
  await stateSet(iii, MESSAGES_SCOPE, session_id, messages);
}
