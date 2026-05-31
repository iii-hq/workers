/**
 * Sync (pre-turn) compaction handler — `compact_now`.
 *
 * Called by the turn-orchestrator before a turn that would overflow.
 * Sequence: lease → prune → summarise → reinject replay → auto-continue.
 */

import { setCurrentSpanAttribute, withSpan } from '@iii-dev/observability';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import { compactionConfig } from './config.js';
import {
  persistCompactionFlatState,
  publishCompactionDone,
  runSummarizeCompaction,
} from './handler-pipeline.js';
import { acquireLeaseWithWait, releaseLease } from './lease.js';
import type { ModelLimit } from './overflow.js';
import { type MessageWithEntryId, extractReplayTarget, reinjectReplay } from './replay.js';

export type CompactNowInput = {
  session_id: string;
  /** TODO(task-27): used by turn-orchestrator pre-flight to decide whether to call compact_now. Accepted now for forward-compat; not consumed by this handler. */
  projected_tokens: number;
  last_user_message_id: string;
  model: { id: string; providerID: string; limit: ModelLimit };
};

export type CompactNowResult =
  | {
      status: 'ok';
      tail_start_id: string | null;
      tokens_before: number;
      auto_continued: boolean;
      summary_text: string;
    }
  | { status: 'busy' }
  | { status: 'overflow'; message: string }
  | { status: 'empty' };

export async function handleSync(iii: ISdk, input: CompactNowInput): Promise<CompactNowResult> {
  return withSpan('compaction.sync', {}, async () => {
    setCurrentSpanAttribute('session_id', input.session_id);

    const leaseStart = Date.now();
    const nonce = await acquireLeaseWithWait(
      iii,
      input.session_id,
      'compaction',
      compactionConfig().busyTimeoutMs,
    );
    const lease_wait_ms = Date.now() - leaseStart;
    setCurrentSpanAttribute('lease_wait_ms', lease_wait_ms);

    if (!nonce) return { status: 'busy' };

    try {
      const resp = await iii.trigger<
        unknown,
        { messages?: Array<{ entry_id?: string; message?: AgentMessage }> }
      >({
        function_id: 'session-tree::messages',
        payload: { session_id: input.session_id },
        timeoutMs: 30_000,
      });
      const entries: MessageWithEntryId[] = (resp?.messages ?? [])
        .filter(
          (e): e is { entry_id: string; message: AgentMessage } =>
            typeof e?.entry_id === 'string' && Boolean(e?.message),
        )
        .map((e) => ({ entry_id: e.entry_id, message: e.message }));

      const { replay, truncatedMessages } = extractReplayTarget(
        entries,
        input.last_user_message_id,
      );

      setCurrentSpanAttribute('replayed', Boolean(replay));

      const result = await runSummarizeCompaction(
        iii,
        input.session_id,
        { mode: 'sync', truncatedEntries: truncatedMessages },
        {
          providerID: input.model.providerID,
          modelID: input.model.id,
          modelLimit: input.model.limit,
        },
      );

      if (result === 'empty') return { status: 'empty' };
      if (result.kind === 'compact') {
        return { status: 'overflow', message: result.reason };
      }

      setCurrentSpanAttribute('tokens_before', result.tokens_before);

      const auto_continued = Boolean(replay);
      setCurrentSpanAttribute('auto_continued', auto_continued);

      let lastEntryId = result.compaction_entry_id || null;
      if (replay) {
        lastEntryId = await reinjectReplay(iii, input.session_id, replay, lastEntryId);
        await iii.trigger<unknown, { entry_id?: string }>({
          function_id: 'session-tree::append_synthetic',
          payload: {
            session_id: input.session_id,
            text: 'Continue if you have next steps, or stop and ask for clarification.',
            metadata: { compaction_continue: true },
            parent_id: lastEntryId,
          },
          timeoutMs: 10_000,
        });
      }

      await persistCompactionFlatState(
        iii,
        input.session_id,
        result.summary_text,
        result.tail_messages,
        replay ? [replay.message] : undefined,
      );

      await publishCompactionDone(iii, input.session_id, 'sync', result);

      return {
        status: 'ok',
        tail_start_id: result.tail_start_id,
        tokens_before: result.tokens_before,
        auto_continued,
        summary_text: result.summary_text,
      };
    } catch (err) {
      logger.warn('handler-sync: sync compaction failed', {
        session_id: input.session_id,
        err: String(err),
      });
      throw err;
    } finally {
      await releaseLease(iii, input.session_id, nonce, 'compaction');
    }
  });
}
