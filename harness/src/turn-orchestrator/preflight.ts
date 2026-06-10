/**
 * Pre-flight overflow check. Returns 'compacted' when compact_now ran;
 * caller must then reload messages from persistence.
 *
 * @throws ContextOverflowError when the session is too large to compact (terminal).
 * @throws CompactionBusyError  when another compaction is in progress — a
 *   TransientError subclass, so the turn-step queue retries the step after
 *   the in-flight compaction releases the lease instead of failing the run.
 */

import { fetchModelLimit, limitFromModel } from '../context-compaction/model-resolver.js';
import { usable as computeUsable } from '../context-compaction/overflow.js';
import type { Model } from '../models-catalog/types.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { AgentMessage } from '../types/agent-message.js';
import { CompactionBusyError, ContextOverflowError } from './errors.js';

/** Cheap chars/4 token estimate — same heuristic as context-compaction's estimateTokenCount. */
function estimateMessages(messages: AgentMessage[]): number {
  let chars = 0;
  for (const m of messages) chars += JSON.stringify(m).length;
  return Math.floor(chars / 4);
}

function findLastUserEntryId(
  entries: Array<{ entry_id?: string; message?: { role?: string } }>,
): string {
  for (let i = entries.length - 1; i >= 0; i--) {
    if (entries[i]?.message?.role === 'user') {
      return entries[i]?.entry_id ?? '';
    }
  }
  return '';
}

export async function runPreflight(
  iii: ISdk,
  session_id: string,
  messages: AgentMessage[],
  providerID: string,
  modelID: string,
  modelMeta?: Model,
): Promise<'ok' | 'compacted'> {
  const model = modelMeta
    ? { providerID, modelID, modelLimit: limitFromModel(modelMeta) }
    : await fetchModelLimit(iii, providerID, modelID);
  if (!model || model.modelLimit.context === 0) {
    if (model && model.modelLimit.context === 0) {
      logger.debug('preflight: model has no context_window; skipping pre-flight check', {
        providerID,
        modelID,
      });
    }
    return 'ok';
  }

  const usable = computeUsable({ model: { id: modelID, limit: model.modelLimit } });
  const projected = estimateMessages(messages);

  if (projected < usable) return 'ok';

  logger.info('preflight: projected overflow detected; triggering compact_now', {
    session_id,
    projected,
    usable,
    model: modelID,
  });

  let lastUserEntryId = '';
  try {
    const resp = await iii.trigger<
      unknown,
      { messages?: Array<{ entry_id?: string; message?: { role?: string } }> }
    >({
      function_id: 'session-tree::messages',
      payload: { session_id },
      timeoutMs: 10_000,
    });
    lastUserEntryId = findLastUserEntryId(resp?.messages ?? []);
  } catch (err) {
    logger.debug('preflight: session-tree::messages failed; using empty last_user_entry_id', {
      session_id,
      err: String(err),
    });
  }

  const res = await iii.trigger<unknown, { status: string }>({
    function_id: 'context-compaction::compact_now',
    payload: {
      session_id,
      projected_tokens: projected,
      last_user_message_id: lastUserEntryId,
      model: { id: model.modelID, providerID: model.providerID, limit: model.modelLimit },
    },
    timeoutMs: 60_000,
  });

  if (res?.status === 'overflow') {
    throw new ContextOverflowError('session too large to compact');
  }
  if (res?.status === 'busy') {
    throw new CompactionBusyError('compaction already in progress');
  }
  if (res?.status !== 'ok') {
    logger.warn('preflight: compact_now returned non-ok status; proceeding without reload', {
      session_id,
      status: res?.status,
    });
    return 'ok';
  }

  return 'compacted';
}
