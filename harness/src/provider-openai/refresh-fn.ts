/**
 * `provider::openai::refresh_models` — re-pull the upstream model list and
 * register the chat-capable subset into the iii models catalog. Returns
 * `{ registered }`. Never throws across the bus boundary.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import type { WorkerConfig } from './config.js';
import { discoverAndRegister } from './discover.js';

export const FUNCTION_ID = 'provider::openai::refresh_models';

export type RefreshResult = { registered: string[] };

export function register(iii: ISdk, worker: WorkerConfig): void {
  iii.registerFunction(
    FUNCTION_ID,
    async (): Promise<RefreshResult> => {
      try {
        return { registered: await discoverAndRegister(iii, worker) };
      } catch (err) {
        logger.warn('provider::openai::refresh_models failed', { err: String(err) });
        return { registered: [] };
      }
    },
    {
      description:
        'Re-pull the OpenAI model list (GET /v1/models) and register the chat-capable subset into the iii models catalog. Idempotent.',
    },
  );
}
