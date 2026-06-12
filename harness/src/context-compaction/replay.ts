import type { AgentMessage } from '../types/agent-message.js';

export type MessageWithEntryId = { entry_id: string; message: AgentMessage };

export function extractReplayTarget(
  entries: MessageWithEntryId[],
  lastUserMessageId: string,
): { replay?: MessageWithEntryId; truncatedMessages: MessageWithEntryId[] } {
  const idx = entries.findIndex((e) => e.entry_id === lastUserMessageId);
  if (idx === -1) return { truncatedMessages: entries };
  const entry = entries[idx];
  if (entry?.message.role !== 'user') return { truncatedMessages: entries };
  return {
    replay: entry,
    truncatedMessages: entries.slice(0, idx),
  };
}
