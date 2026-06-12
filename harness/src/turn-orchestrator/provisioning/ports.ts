/**
 * Typed dependency ports for provisioning.
 */

import type { Model } from '../../models-catalog/types.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { getCatalogModel } from '../../runtime/output-tokens.js';
import type { RunRequest } from '../run-request.js';
import { createTurnStore } from '../state-runtime/store.js';

const ROUTE_TIMEOUT_MS = 5_000;

export type ProvisioningPorts = {
  loadRunRequest(session_id: string): Promise<RunRequest>;
  saveRunRequest(session_id: string, request: RunRequest): Promise<void>;
  /**
   * Preview the routing decision via `router::route`; null when the router is
   * unreachable or nothing routes (the chat call then omits `provider` and
   * the router re-decides server-side — failing loudly if it's still down).
   */
  route(provider: string, model: string): Promise<string | null>;
  /** Resolve the full catalog entry for (provider, model); null on miss/error. */
  resolveModel(provider: string, model: string): Promise<Model | null>;
};

export function createProvisioningPorts(iii: ISdk): ProvisioningPorts {
  const store = createTurnStore(iii);

  return {
    loadRunRequest(session_id) {
      return store.loadRunRequest(session_id);
    },

    saveRunRequest(session_id, request) {
      return store.saveRunRequest(session_id, request);
    },

    async route(provider, model) {
      try {
        const res = await iii.trigger<unknown, { provider?: string }>({
          function_id: 'router::route',
          payload: { model, ...(provider ? { provider } : {}) },
          timeoutMs: ROUTE_TIMEOUT_MS,
        });
        return typeof res?.provider === 'string' && res.provider.length > 0 ? res.provider : null;
      } catch (err) {
        logger.warn('provisioning: router::route failed', {
          provider,
          model,
          err: String(err),
        });
        return null;
      }
    },

    resolveModel(provider, model) {
      return getCatalogModel(iii, provider, model);
    },
  };
}
