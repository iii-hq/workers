/**
 * Cheap chars/4 token estimate used for pre-flight overflow detection.
 * Same heuristic as context-compaction's estimateTokenCount.
 */

import type { AgentMessage } from '../types/agent-message.js';

export function estimateMessages(messages: AgentMessage[]): number {
  let chars = 0;
  for (const m of messages) chars += JSON.stringify(m).length;
  return Math.floor(chars / 4);
}
