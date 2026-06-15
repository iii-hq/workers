/**
 * Read helper for the llm-router model catalog. Output-token budgeting moved
 * into the router itself (override > model ceiling > soft cap); the harness
 * only reads catalog entries for preflight sizing and provisioning metadata.
 */

import type { Model } from '../types/model.js';
import type { ISdk } from './iii.js';
import { logger } from './otel.js';

const MODELS_GET_TIMEOUT_MS = 5_000;

/**
 * Fetch the catalog entry for `(provider, model)` via `router::models::get`
 * (payload key is `id`, the result is wrapped as `{ model }`, null on miss).
 * Best-effort: returns null on miss, timeout, or any bus error — callers fall
 * back to defaults so an empty catalog never breaks a request.
 */
export async function getCatalogModel(
  iii: ISdk,
  provider: string,
  modelId: string,
): Promise<Model | null> {
  try {
    const entry = await iii.trigger<unknown, { model?: Model | null } | null>({
      function_id: 'router::models::get',
      payload: { provider, id: modelId },
      timeoutMs: MODELS_GET_TIMEOUT_MS,
    });
    return entry?.model ?? null;
  } catch (err) {
    logger.debug('catalog: router::models::get failed', {
      provider,
      model: modelId,
      err: String(err),
    });
    return null;
  }
}
