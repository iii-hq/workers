/**
 * `provider::lmstudio::load_model` — bus function the UI / orchestrator
 * calls to load a model into LM Studio's memory on demand. Wraps the
 * native `POST /api/v1/models/load` endpoint.
 *
 * Typical use:
 *   - UI calls this when the user picks a model from the dropdown that
 *     isn't yet loaded, so streaming starts immediately afterwards.
 *   - The stream handler also auto-calls the underlying loader on first
 *     use for a not-yet-loaded model; this bus function is the explicit
 *     "preload now" knob.
 *
 * Returns the LM Studio load result on success, or
 * `{ ok: false, error: string }` on failure — never throws across the
 * bus boundary so a UI prefetch failure doesn't crash the picker.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { buildAuthHeaders } from './auth.js';
import type { WorkerConfig } from './config.js';
import { type LoadOptions, type LoadResult, loadModel } from './load.js';

export const FUNCTION_ID = 'provider::lmstudio::load_model';

export type LoadModelPayload = {
  model: string;
} & LoadOptions;

export type LoadModelResult = LoadResult | { ok: false; error: string };

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (raw: unknown): Promise<LoadModelResult> => {
      const payload = raw as Partial<LoadModelPayload> | null | undefined;
      if (!payload || typeof payload.model !== 'string' || payload.model.length === 0) {
        return { ok: false, error: 'invalid payload: { model: string } is required' };
      }
      try {
        const headers = await buildAuthHeaders(iii);
        return await loadModel(worker.default_api_url, headers, payload.model, {
          context_length: payload.context_length,
          eval_batch_size: payload.eval_batch_size,
          flash_attention: payload.flash_attention,
          num_experts: payload.num_experts,
          offload_kv_cache_to_gpu: payload.offload_kv_cache_to_gpu,
          echo_load_config: payload.echo_load_config,
        });
      } catch (err) {
        logger.warn('provider::lmstudio::load_model failed', {
          model: payload.model,
          err: String(err),
        });
        return { ok: false, error: String(err) };
      }
    },
    {
      description:
        'Load an LM Studio model into memory via POST /api/v1/models/load. Blocks until ready (up to 120s).',
    },
  );
}
