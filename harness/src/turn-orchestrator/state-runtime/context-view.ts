/**
 * Reconstruct the provider-facing message window from session-manager state.
 *
 * No compaction: the raw active path. With compaction: the latest summary
 * followed by the preserved tail (everything from `tail_start_id` onward).
 */

import { buildSummaryMessage } from '../../context-compaction/flat-state.js';
import type { ISdk } from '../../runtime/iii.js';
import {
  compactionRowsFromPath,
  messageItemsFromPath,
  readActivePath,
  type CompactionRow,
  type MessageWithEntryId,
} from '../../runtime/session.js';
import type { AgentMessage } from '../../types/agent-message.js';

export type ContextViewCompaction = Pick<CompactionRow, 'summary' | 'tail_start_id' | 'timestamp'>;

export type ContextViewOptions = {
  /**
   * Entry ids excluded from the window — this turn's just-appended assistant
   * placeholder on step re-entry, which must not be replayed to the provider.
   */
  excludeEntryIds?: string[];
};

function latestCompaction(compactions: ContextViewCompaction[]): ContextViewCompaction | null {
  if (compactions.length === 0) return null;
  // Stable sort: ties keep path order, which is the authoritative order.
  return [...compactions].sort((a, b) => a.timestamp - b.timestamp).at(-1) ?? null;
}

/**
 * Empty assistant messages are streaming placeholders (or the residue of a
 * run that died before its first delta); providers reject empty assistant
 * turns, so they never enter the window.
 */
function isEmptyAssistant(m: AgentMessage): boolean {
  return m.role === 'assistant' && m.content.length === 0;
}

/** Pure reconstruction from path-ordered messages and compaction rows. */
export function buildContextView(
  messages: MessageWithEntryId[],
  compactions: ContextViewCompaction[],
): AgentMessage[] {
  const usable = messages.filter((m) => !isEmptyAssistant(m.message));
  const compaction = latestCompaction(compactions);
  if (!compaction) {
    return usable.map((m) => m.message);
  }
  const found = compaction.tail_start_id
    ? usable.findIndex((m) => m.entry_id === compaction.tail_start_id)
    : 0;
  const tailStart = found >= 0 ? found : 0;
  return [
    buildSummaryMessage(compaction.summary),
    ...usable.slice(tailStart).map((m) => m.message),
  ];
}

export async function loadContextView(
  iii: ISdk,
  session_id: string,
  opts?: ContextViewOptions,
): Promise<AgentMessage[]> {
  const items = await readActivePath(iii, session_id, { include_custom: true });
  const exclude = new Set(opts?.excludeEntryIds ?? []);
  const entries = messageItemsFromPath(items).filter((e) => !exclude.has(e.entry_id));
  const compactions = compactionRowsFromPath(items);
  return buildContextView(entries, compactions);
}
