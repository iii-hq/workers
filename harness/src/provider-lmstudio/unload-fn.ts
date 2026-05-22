/**
 * `provider::lmstudio::unload_model` — bus function to release a loaded
 * LM Studio model instance. Wraps `POST /api/v1/models/unload`.
 *
 * Pairs with `provider::lmstudio::load_model`. Useful for freeing memory
 * before loading a different model on RAM-constrained hosts, or as part
 * of an explicit teardown.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { buildAuthHeaders } from './auth.js';
import type { WorkerConfig } from './config.js';
import { type UnloadResult, unloadModel } from './load.js';

export const FUNCTION_ID = 'provider::lmstudio::unload_model';

export type UnloadModelPayload = {
  instance_id: string;
};

export type UnloadModelResult = UnloadResult | { ok: false; error: string };

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (raw: unknown): Promise<UnloadModelResult> => {
      const payload = raw as Partial<UnloadModelPayload> | null | undefined;
      if (
        !payload ||
        typeof payload.instance_id !== 'string' ||
        payload.instance_id.length === 0
      ) {
        return { ok: false, error: 'invalid payload: { instance_id: string } is required' };
      }
      try {
        const headers = await buildAuthHeaders(iii, worker.default_api_url);
        return await unloadModel(worker.default_api_url, headers, payload.instance_id);
      } catch (err) {
        logger.warn('provider::lmstudio::unload_model failed', {
          instance_id: payload.instance_id,
          err: String(err),
        });
        return { ok: false, error: String(err) };
      }
    },
    {
      description:
        'Unload an LM Studio model instance via POST /api/v1/models/unload.',
    },
  );
}
