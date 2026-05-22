/**
 * `provider::lmstudio::refresh_models` — bus function the UI (or a script)
 * can call to re-discover loaded LM Studio models without restarting the
 * worker. Wraps `discoverAndRegister`.
 *
 * Returns `{ registered: string[] }` — the IDs of all models that were
 * (re-)written into the catalog on this call. Idempotent: re-registering
 * the same model is a no-op state set.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { buildAuthHeaders } from './auth.js';
import type { WorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';

export const FUNCTION_ID = 'provider::lmstudio::refresh_models';

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
        // Never throw across the bus boundary — refresh is a best-effort
        // utility and a failed refresh shouldn't tank the calling UI.
        logger.warn('provider::lmstudio::refresh_models failed', {
          err: String(err),
        });
        return { registered: [] };
      }
    },
    {
      description:
        'Re-discover loaded LM Studio models and register each into the iii models catalog. Idempotent.',
    },
  );
}
