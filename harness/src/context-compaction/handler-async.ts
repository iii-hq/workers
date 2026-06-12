/**
 * Async (turn_end-driven) compaction. Woken by a turn-orchestrator queue
 * message (`context-compaction::on_turn_end`) carrying a typed
 * `{ session_id, usage, provider, model, model_limit? }` payload.
 */

import { setCurrentSpanAttribute, withSpan } from '@iii-dev/observability';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { readActivePath } from '../runtime/session.js';
import type { ModelContextLimit } from '../types/agent-event.js';
import type { Usage } from '../types/stream-event.js';
import { compactionConfig } from './config.js';
import {
  isSummarizeOk,
  publishCompactionDone,
  runSummarizeCompaction,
} from './handler-pipeline.js';
import { acquireLease, releaseLease } from './lease.js';
import { fetchModelLimit } from './model-resolver.js';
import { isOverflow } from './overflow.js';

export type OnTurnEndPayload = {
  session_id: string;
  usage: Usage | null;
  provider: string;
  model: string;
  /**
   * Threaded from the turn record's `model_meta` when the catalog resolved at
   * turn start; lets the compactor skip its `models::get` fetch.
   */
  model_limit?: ModelContextLimit;
};

/** Parse the queue payload enqueued by turn-orchestrator at turn_end. */
export function parseOnTurnEnd(payload: unknown): OnTurnEndPayload | null {
  if (!payload || typeof payload !== 'object') return null;
  const obj = payload as Record<string, unknown>;
  if (typeof obj.session_id !== 'string' || !obj.session_id) return null;
  return {
    session_id: obj.session_id,
    usage: obj.usage && typeof obj.usage === 'object' ? (obj.usage as Usage) : null,
    provider: typeof obj.provider === 'string' ? obj.provider : '',
    model: typeof obj.model === 'string' ? obj.model : '',
    ...(obj.model_limit && typeof obj.model_limit === 'object'
      ? { model_limit: obj.model_limit as ModelContextLimit }
      : {}),
  };
}

type ResolvedModel = {
  providerID: string;
  modelID: string;
  modelLimit: ModelContextLimit;
} | null;

/**
 * Resolve the model limit from the payload's provider/model, falling back to
 * the session's most recent assistant message when either is missing. When the
 * payload carries a threaded `model_limit` (catalog resolved at turn start),
 * the `models::get` fetch is skipped entirely.
 */
export async function resolveModel(
  iii: ISdk,
  session_id: string,
  provider: string,
  model: string,
  model_limit?: ModelContextLimit,
): Promise<ResolvedModel> {
  let providerID: string | null = provider || null;
  let modelID: string | null = model || null;

  if (providerID && modelID && model_limit) {
    return { providerID, modelID, modelLimit: model_limit };
  }

  if (!providerID || !modelID) {
    try {
      const items = await readActivePath(iii, session_id, { roles: ['assistant'] });
      for (let i = items.length - 1; i >= 0; i--) {
        const m = items[i]?.message as Record<string, unknown> | undefined;
        if (!m) continue;
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

export async function handleAsync(iii: ISdk, payload: unknown): Promise<void> {
  return withSpan('compaction::async', {}, async () => {
    const parsed = parseOnTurnEnd(payload);
    if (!parsed?.usage) return;
    const { session_id, usage } = parsed;

    setCurrentSpanAttribute('session_id', session_id);

    const model = await resolveModel(
      iii,
      session_id,
      parsed.provider,
      parsed.model,
      parsed.model_limit,
    );
    if (!model) {
      logger.debug('handler-async: could not resolve model; skipping overflow check', {
        session_id,
      });
      return;
    }

    const tokens_before =
      (usage.input ?? 0) + (usage.output ?? 0) + (usage.cache_read ?? 0) + (usage.cache_write ?? 0);
    setCurrentSpanAttribute('tokens_before', tokens_before);

    if (
      !isOverflow({
        tokens: usage,
        model: { id: model.modelID, limit: model.modelLimit },
        reserved: compactionConfig().reservedTokens,
      })
    ) {
      return;
    }

    const nonce = await acquireLease(iii, session_id, 'compaction');
    if (!nonce) {
      logger.debug('handler-async: compaction lease held; skipping', { session_id });
      return;
    }

    try {
      const result = await runSummarizeCompaction(iii, session_id, { mode: 'async' }, model);

      setCurrentSpanAttribute('used_prior_summary', isSummarizeOk(result));
      if (isSummarizeOk(result)) {
        await publishCompactionDone(iii, session_id, 'async', result);
      }
    } catch (err) {
      logger.warn('handler-async: compaction failed', { session_id, err: String(err) });
    } finally {
      await releaseLease(iii, session_id, nonce, 'compaction');
    }
  });
}
