/**
 * `web::fetch` handler. Validates the payload, runs the SSRF-guarded
 * HTTP call, and returns the structured envelope. Never throws — the
 * envelope is the contract.
 */

import type { ISdk } from '../../runtime/iii.js';
import type { WebConfig } from '../config.js';
import { executeFetch } from '../fetch.js';
import {
  type FetchImageResult,
  FetchPayloadSchema,
  type FetchResult,
  fetchFunctionOptions,
} from '../schemas.js';

export function register(iii: ISdk, cfg: WebConfig): void {
  iii.registerFunction(
    'web::fetch',
    async (payload: unknown): Promise<FetchResult | FetchImageResult> => {
      const parsed = FetchPayloadSchema.safeParse(payload);
      if (!parsed.success) {
        return {
          ok: false,
          error: 'invalid_payload',
          message: parsed.error.issues
            .map((i) => `${i.path.join('.') || '<root>'}: ${i.message}`)
            .join('; '),
        };
      }
      return executeFetch(parsed.data, cfg);
    },
    fetchFunctionOptions,
  );
}
