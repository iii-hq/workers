import type { ISdk } from '../runtime/iii.js';
import type { AgentMessage } from '../types/agent-message.js';

export type MessageWithEntryId = { entry_id: string; message: AgentMessage };

export function extractReplayTarget(
  entries: MessageWithEntryId[],
  lastUserMessageId: string,
): { replay?: MessageWithEntryId; truncatedMessages: MessageWithEntryId[] } {
  const idx = entries.findIndex((e) => e.entry_id === lastUserMessageId);
  if (idx === -1) return { truncatedMessages: entries };
  const entry = entries[idx];
  if (!entry || entry.message.role !== 'user') return { truncatedMessages: entries };
  return {
    replay: entry,
    truncatedMessages: entries.slice(0, idx),
  };
}

export async function reinjectReplay(
  iii: ISdk,
  session_id: string,
  replay: MessageWithEntryId,
  parent_id: string | null,
): Promise<string | null> {
  const resp = await iii.trigger<unknown, { entry_id?: string }>({
    function_id: 'session-tree::append',
    payload: {
      session_id,
      parent_id,
      message: replay.message,
    },
    timeoutMs: 10_000,
  });
  return resp?.entry_id ?? parent_id;
}
