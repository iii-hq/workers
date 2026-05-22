/**
 * `provider::llamacpp::refresh_models` — bus function the UI (or a
 * script) can call to re-discover the loaded llama-server model
 * without restarting this worker. Wraps `discoverAndRegister`.
 *
 * Returns `{ registered: string[] }` — the IDs (typically one, since
 * llama-server hosts a single model per process) of all models that
 * were (re-)written into the catalog on this call. Idempotent.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { buildAuthHeaders } from './auth.js';
import type { WorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';

export const FUNCTION_ID = 'provider::llamacpp::refresh_models';

export type RefreshResult = {
  registered: string[];
};

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (): Promise<RefreshResult> => {
      try {
        const headers = await buildAuthHeaders(iii, worker.default_api_url);
        const registered = await discoverAndRegister(
          iii,
          worker.default_api_url,
          headers,
        );
        return { registered };
      } catch (err) {
        // Never throw across the bus boundary — refresh is best-effort.
        logger.warn('provider::llamacpp::refresh_models failed', {
          err: String(err),
        });
        return { registered: [] };
      }
    },
    {
      description:
        'Re-discover the loaded llama-server model and register it into the iii models catalog. Idempotent.',
    },
  );
}
