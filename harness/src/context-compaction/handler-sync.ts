/**
 * Sync (pre-turn) compaction handler — `compact_now`.
 *
 * Called by the turn-orchestrator before a turn that would overflow.
 * Sequence: lease → prune → summarise → reinject replay → auto-continue.
 */

import { setCurrentSpanAttribute, withSpan } from 'iii-sdk/telemetry';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { emit } from '../turn-orchestrator/events.js';
import type { AgentMessage } from '../types/agent-message.js';
import { busyTimeoutMs, pruneMinFree, pruneProtect, pruneProtectedTools } from './config.js';
import { buildSummaryMessage, rewriteFlatMessages } from './flat-state.js';
import { acquireLeaseWithWait, releaseLease } from './lease.js';
import type { ModelLimit } from './overflow.js';
import { prune } from './prune.js';
import { type MessageWithEntryId, extractReplayTarget, reinjectReplay } from './replay.js';
import { summarizeAndAppend } from './summarize.js';

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
    const nonce = await acquireLeaseWithWait(iii, input.session_id, 'compaction', busyTimeoutMs());
    const lease_wait_ms = Date.now() - leaseStart;
    setCurrentSpanAttribute('lease_wait_ms', lease_wait_ms);

    if (!nonce) return { status: 'busy' };

    try {
      // Load entries with IDs from the session tree
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

      // Prune older tool outputs first (cheap path)
      await prune(iii, input.session_id, {
        protectTokens: pruneProtect(),
        minFree: pruneMinFree(),
        protectedTools: pruneProtectedTools(),
      });

      const result = await summarizeAndAppend(
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
      // tokens_kept attribute deferred — currently identical to tokens_before.

      const auto_continued = Boolean(replay);
      setCurrentSpanAttribute('auto_continued', auto_continued);

      // Chain parent_id on the tree appends so the active path stays
      // connected: Compaction -> replay user msg -> synthetic continue.
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

      // Rewrite the flat state used by the turn-orchestrator's provider
      // input. Without this, persistence.loadMessages still returns the
      // pre-compaction history and the next provider call overflows.
      const new_flat_messages: AgentMessage[] = [
        buildSummaryMessage(result.summary_text),
        ...result.tail_messages,
      ];
      if (replay) new_flat_messages.push(replay.message);
      await rewriteFlatMessages(iii, input.session_id, new_flat_messages);

      // Tell the UI we just compacted so it can render a marker and
      // re-estimate context usage. Best-effort: a publish failure must
      // not derail the in-flight turn — the orchestrator is waiting on
      // our return.
      try {
        await emit(iii, input.session_id, {
          type: 'compaction_done',
          mode: 'sync',
          summary_text: result.summary_text,
          tokens_before: result.tokens_before,
          compaction_entry_id: result.compaction_entry_id,
          tail_start_id: result.tail_start_id,
        });
      } catch (err) {
        logger.warn('handler-sync: compaction_done emit failed', {
          session_id: input.session_id,
          err: String(err),
        });
      }

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
