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

type CompactionsResponse = {
  entries?: CompactionEntryRow[];
};

export type SessionMessageRow = { entry_id: string; message: AgentMessage };

/**
 * Raw active-path messages (entry_id + message), oldest first. The single
 * `session-tree::messages` reader — shared by {@link loadContextView} (which
 * also folds in compactions) and the orchestrator's trailing-result dedup — so
 * the fetch shape and timeout live in one place.
 */
export async function loadSessionMessages(
  iii: ISdk,
  session_id: string,
): Promise<SessionMessageRow[]> {
  const resp = await iii.trigger<unknown, { messages?: SessionMessageRow[] }>({
    function_id: 'session-tree::messages',
    payload: { session_id },
    // Matches the SDK default this read always ran under: long sessions
    // (thousands of entries) can legitimately take >10s to list+sort, and a
    // tighter budget here fails the whole turn step.
    timeoutMs: 30_000,
  });
  return resp?.messages ?? [];
}

export async function loadContextView(iii: ISdk, session_id: string): Promise<AgentMessage[]> {
  const [messages, compactionsResp] = await Promise.all([
    loadSessionMessages(iii, session_id),
    iii.trigger({ function_id: 'session-tree::compactions', payload: { session_id } }),
  ]);

  const compactionsPayload = compactionsResp as CompactionsResponse | null;

  const entries: MessageWithEntryId[] = messages.map((e) => ({
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
