import { setCurrentSpanAttribute, withSpan } from '@iii-dev/observability';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { messageItemsFromPath, readActivePath, sessionUpdateMessage } from '../runtime/session.js';
import type { AgentMessage } from '../types/agent-message.js';

export type PruneOptions = {
  protectTokens: number;
  minFree: number;
  protectedTools: string[];
};

export type PruneResult = {
  pruned_tokens: number;
  pruned_parts: number;
  scanned_parts: number;
};

function tokensOfText(text: string): number {
  return Math.floor(text.length / 4);
}

export async function prune(
  iii: ISdk,
  session_id: string,
  opts: PruneOptions,
): Promise<PruneResult> {
  return withSpan('compaction.prune', {}, async () => {
    setCurrentSpanAttribute('session_id', session_id);

    const entries = messageItemsFromPath(await readActivePath(iii, session_id));
    if (entries.length === 0) {
      setCurrentSpanAttribute('pruned_tokens', 0);
      setCurrentSpanAttribute('pruned_parts', 0);
      setCurrentSpanAttribute('scanned_parts', 0);
      return { pruned_tokens: 0, pruned_parts: 0, scanned_parts: 0 };
    }

    let total = 0;
    let scanned = 0;
    let userTurns = 0;
    const queue: Array<{ entry_id: string; tokens: number; message: AgentMessage }> = [];

    for (let i = entries.length - 1; i >= 0; i--) {
      const e = entries[i];
      if (!e) continue;
      if (e.message.role === 'user') {
        userTurns++;
        continue;
      }
      if (userTurns < 2) continue;
      if (e.message.role !== 'function_result') continue;
      if (opts.protectedTools.includes(e.message.function_id)) continue;
      const text = e.message.content
        .filter(
          (b): b is { type: 'text'; text: string } => (b as { type?: string }).type === 'text',
        )
        .map((b) => b.text)
        .join('');
      const t = tokensOfText(text);
      scanned++;
      total += t;
      if (total <= opts.protectTokens) continue;
      queue.push({ entry_id: e.entry_id, tokens: t, message: e.message });
    }

    const pruned_tokens = queue.reduce((a, q) => a + q.tokens, 0);
    if (pruned_tokens < opts.minFree) {
      logger.debug('prune skipped (not enough to free)', { session_id, pruned_tokens, scanned });
      setCurrentSpanAttribute('pruned_tokens', 0);
      setCurrentSpanAttribute('pruned_parts', 0);
      setCurrentSpanAttribute('scanned_parts', scanned);
      return { pruned_tokens: 0, pruned_parts: 0, scanned_parts: scanned };
    }

    const compacted_at = Date.now();
    for (const q of queue) {
      // update_message replaces `details` wholesale for function_result
      // entries, so merge the existing details with the compaction stamp.
      const existing =
        q.message.role === 'function_result' &&
        q.message.details &&
        typeof q.message.details === 'object'
          ? (q.message.details as Record<string, unknown>)
          : {};
      try {
        await sessionUpdateMessage(iii, {
          session_id,
          entry_id: q.entry_id,
          content: [{ type: 'text', text: '[output pruned]' }],
          details: { ...existing, compacted_at },
        });
      } catch (err) {
        logger.warn('prune: session::update_message failed; skipping entry', {
          session_id,
          entry_id: q.entry_id,
          err: String(err),
        });
      }
    }

    logger.info('pruned tool outputs', { session_id, pruned_tokens, pruned_parts: queue.length });
    setCurrentSpanAttribute('pruned_tokens', pruned_tokens);
    setCurrentSpanAttribute('pruned_parts', queue.length);
    setCurrentSpanAttribute('scanned_parts', scanned);
    return { pruned_tokens, pruned_parts: queue.length, scanned_parts: scanned };
  });
}
