/**
 * Build the assistant summary message used in the compacted provider window.
 */

import type { AssistantMessage } from '../types/agent-message.js';

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
