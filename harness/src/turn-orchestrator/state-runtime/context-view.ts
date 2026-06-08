/**
 * Reconstruct the provider-facing message window from session-tree state.
 *
 * No compaction: the raw active path. With compaction: the latest summary
 * followed by the preserved tail (everything from `tail_start_id` onward).
 */

import type { CompactionEntryRow } from '../../session/tree/operations.js';
import type { MessageWithEntryId } from '../../session/tree/types.js';
import { buildSummaryMessage } from '../../context-compaction/flat-state.js';
import type { AgentMessage } from '../../types/agent-message.js';
import type { ISdk } from '../../runtime/iii.js';

export type ContextViewCompaction = Pick<
  CompactionEntryRow,
  'summary' | 'tail_start_id' | 'timestamp'
>;

function latestCompaction(compactions: ContextViewCompaction[]): ContextViewCompaction | null {
  if (compactions.length === 0) return null;
  return [...compactions].sort((a, b) => a.timestamp - b.timestamp).at(-1) ?? null;
}

/** Pure reconstruction from path-ordered messages and compaction rows. */
export function buildContextView(
  messages: MessageWithEntryId[],
  compactions: ContextViewCompaction[],
): AgentMessage[] {
  const compaction = latestCompaction(compactions);
  if (!compaction) {
    return messages.map((m) => m.message);
  }
  const found = compaction.tail_start_id
    ? messages.findIndex((m) => m.entry_id === compaction.tail_start_id)
    : 0;
  const tailStart = found >= 0 ? found : 0;
  return [
    buildSummaryMessage(compaction.summary),
    ...messages.slice(tailStart).map((m) => m.message),
  ];
}

type MessagesResponse = {
  messages?: Array<{ entry_id: string; message: AgentMessage }>;
};

type CompactionsResponse = {
  entries?: CompactionEntryRow[];
};

export async function loadContextView(iii: ISdk, session_id: string): Promise<AgentMessage[]> {
  const [messagesResp, compactionsResp] = await Promise.all([
    iii.trigger({ function_id: 'session-tree::messages', payload: { session_id } }),
    iii.trigger({ function_id: 'session-tree::compactions', payload: { session_id } }),
  ]);

  const messagesPayload = messagesResp as MessagesResponse | null;
  const compactionsPayload = compactionsResp as CompactionsResponse | null;

  const entries: MessageWithEntryId[] = (messagesPayload?.messages ?? []).map((e) => ({
    entry_id: e.entry_id,
    message: e.message,
  }));

  const compactions: ContextViewCompaction[] = (compactionsPayload?.entries ?? []).map((c) => ({
    summary: c.summary,
    tail_start_id: c.tail_start_id,
    timestamp: c.timestamp,
  }));

  return buildContextView(entries, compactions);
}
