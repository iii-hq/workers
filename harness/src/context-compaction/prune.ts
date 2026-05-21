import { setCurrentSpanAttribute, withSpan } from 'iii-sdk/telemetry';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
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

    const resp = await iii.trigger<
      unknown,
      { messages?: Array<{ entry_id?: string; message?: AgentMessage }> }
    >({
      function_id: 'session-tree::messages',
      payload: { session_id },
      timeoutMs: 10_000,
    });
    const entries = (resp?.messages ?? []).filter(
      (e): e is { entry_id: string; message: AgentMessage } =>
        typeof e?.entry_id === 'string' && Boolean(e?.message),
    );
    if (entries.length === 0) {
      setCurrentSpanAttribute('pruned_tokens', 0);
      setCurrentSpanAttribute('pruned_parts', 0);
      setCurrentSpanAttribute('scanned_parts', 0);
      return { pruned_tokens: 0, pruned_parts: 0, scanned_parts: 0 };
    }

    let total = 0;
    let scanned = 0;
    let userTurns = 0;
    const queue: Array<{ entry_id: string; tokens: number }> = [];

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
      queue.push({ entry_id: e.entry_id, tokens: t });
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
    await iii.trigger<unknown, { updated?: number }>({
      function_id: 'session-tree::update_parts',
      payload: {
        session_id,
        items: queue.map((q) => ({ entry_id: q.entry_id, output: null, compacted_at })),
      },
      timeoutMs: 10_000,
    });

    logger.info('pruned tool outputs', { session_id, pruned_tokens, pruned_parts: queue.length });
    setCurrentSpanAttribute('pruned_tokens', pruned_tokens);
    setCurrentSpanAttribute('pruned_parts', queue.length);
    setCurrentSpanAttribute('scanned_parts', scanned);
    return { pruned_tokens, pruned_parts: queue.length, scanned_parts: scanned };
  });
}
