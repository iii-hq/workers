/**
 * Async (TurnEnd-driven) compaction. Mirrors Rust lib.rs::handle_event.
 * Accepts both camelCase ({groupId, event:{data}}) and snake_case
 * ({group_id, data}) envelopes.
 */

import { setCurrentSpanAttribute, withSpan } from '@iii-dev/observability';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { compactionConfig } from './config.js';
import {
  isSummarizeOk,
  persistCompactionFlatState,
  publishCompactionDone,
  runSummarizeCompaction,
} from './handler-pipeline.js';
import { acquireLease, releaseLease } from './lease.js';
import { fetchModelLimit } from './model-resolver.js';
import { isOverflow } from './overflow.js';

export function extractEventPayload(
  payload: unknown,
): { session_id: string; event: unknown } | null {
  if (!payload || typeof payload !== 'object') return null;
  const obj = payload as Record<string, unknown>;
  const session_id =
    (typeof obj.groupId === 'string' && obj.groupId) ||
    (typeof obj.group_id === 'string' && obj.group_id) ||
    null;
  if (!session_id) return null;
  let event: unknown = null;
  if (
    obj.event &&
    typeof obj.event === 'object' &&
    'data' in (obj.event as Record<string, unknown>)
  ) {
    event = (obj.event as Record<string, unknown>).data;
  } else if ('data' in obj) {
    event = obj.data;
  }
  return { session_id, event };
}

export function turnEndUsage(event: unknown): Record<string, unknown> | null {
  if (!event || typeof event !== 'object') return null;
  const obj = event as Record<string, unknown>;
  const kind = typeof obj.type === 'string' ? obj.type : null;
  if (kind !== 'TurnEnd' && kind !== 'turn_end') return null;
  const msg = obj.message as Record<string, unknown> | undefined;
  const usage = msg?.usage;
  if (!usage || typeof usage !== 'object') return null;
  return usage as Record<string, unknown>;
}

type ResolvedModel = {
  providerID: string;
  modelID: string;
  modelLimit: { context: number; input: number; output: number };
} | null;

async function resolveModelFromEvent(
  iii: ISdk,
  session_id: string,
  event: unknown,
): Promise<ResolvedModel> {
  let providerID: string | null = null;
  let modelID: string | null = null;

  if (event && typeof event === 'object') {
    const ev = event as Record<string, unknown>;
    const msg = ev.message as Record<string, unknown> | undefined;
    if (msg) {
      if (typeof msg.provider === 'string' && msg.provider) providerID = msg.provider;
      if (typeof msg.model === 'string' && msg.model) modelID = msg.model;
    }
  }

  if (!providerID || !modelID) {
    try {
      const resp = await iii.trigger<
        unknown,
        { messages?: Array<{ entry_id?: string; message?: Record<string, unknown> }> }
      >({
        function_id: 'session-tree::messages',
        payload: { session_id },
        timeoutMs: 10_000,
      });
      const messages = resp?.messages ?? [];
      for (let i = messages.length - 1; i >= 0; i--) {
        const m = messages[i]?.message;
        if (!m) continue;
        if (m.role !== 'assistant') continue;
        if (!providerID && typeof m.provider === 'string' && m.provider) {
          providerID = m.provider;
        }
        if (!modelID && typeof m.model === 'string' && m.model) {
          modelID = m.model;
        }
        if (providerID && modelID) break;
      }
    } catch (err) {
      logger.debug('handler-async: failed to load session messages for model resolution', {
        session_id,
        err: String(err),
      });
    }
  }

  if (!providerID || !modelID) return null;
  return fetchModelLimit(iii, providerID, modelID);
}

export async function handleAsync(iii: ISdk, frame: unknown): Promise<void> {
  return withSpan('compaction.async', {}, async () => {
    const payload = extractEventPayload(frame);
    if (!payload) return;

    const usage = turnEndUsage(payload.event);
    if (!usage) return;

    setCurrentSpanAttribute('session_id', payload.session_id);

    const model = await resolveModelFromEvent(iii, payload.session_id, payload.event);
    if (!model) {
      logger.debug('handler-async: could not resolve model; skipping overflow check', {
        session_id: payload.session_id,
      });
      return;
    }

    const usageObj = usage as {
      input?: number;
      output?: number;
      cache_read?: number;
      cache_write?: number;
    };
    const tokens_before =
      (usageObj.input ?? 0) +
      (usageObj.output ?? 0) +
      (usageObj.cache_read ?? 0) +
      (usageObj.cache_write ?? 0);
    setCurrentSpanAttribute('tokens_before', tokens_before);

    if (
      !isOverflow({
        tokens: usageObj,
        model: { id: model.modelID, limit: model.modelLimit },
        reserved: compactionConfig().reservedTokens,
      })
    ) {
      return;
    }

    const nonce = await acquireLease(iii, payload.session_id, 'compaction');
    if (!nonce) {
      logger.debug('handler-async: compaction lease held; skipping', {
        session_id: payload.session_id,
      });
      return;
    }

    try {
      const result = await runSummarizeCompaction(
        iii,
        payload.session_id,
        { mode: 'async' },
        model,
      );

      setCurrentSpanAttribute('used_prior_summary', isSummarizeOk(result));
      if (isSummarizeOk(result)) {
        await persistCompactionFlatState(
          iii,
          payload.session_id,
          result.summary_text,
          result.tail_messages,
        );
        await publishCompactionDone(iii, payload.session_id, 'async', result);
      }
    } catch (err) {
      logger.warn('handler-async: compaction failed', {
        session_id: payload.session_id,
        err: String(err),
      });
    } finally {
      await releaseLease(iii, payload.session_id, nonce, 'compaction');
    }
  });
}
