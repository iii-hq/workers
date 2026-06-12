/**
 * Resolve the Anthropic credential + runtime settings from the llm-router
 * registry (token-gated `router::provider::resolve`), then turn them into an
 * AnthropicConfig.
 */

import type { Model } from '../types/model.js';
import type { ISdk } from '../runtime/iii.js';
import { clampOutputTokens, getCatalogModel } from '../runtime/output-tokens.js';
import {
  type ProviderResolveResult,
  resolveProviderViaRouter,
} from '../runtime/provider-resolve.js';
import type { WorkerConfig } from './config.js';
import { type AnthropicConfig, configWithCredential } from './types.js';

export const PROVIDER_ID = 'anthropic';

/**
 * Single-slot cache of the resolved provider credential, keyed by the turn's
 * stable id (the router's request_id). The credential is global, so within a
 * turn its 20+ stream calls reuse one resolution; a new turn (new key)
 * re-resolves, picking up a key rotated between turns.
 * {@link invalidateProviderResolveCache} drops it on a mid-turn 401. With no
 * key threaded, callers always resolve (no caching).
 */
let resolveCache: { key: string | number; resolved: ProviderResolveResult } | null = null;

async function resolveProviderForTurn(
  iii: ISdk,
  key?: string | number,
): Promise<ProviderResolveResult> {
  if (key === undefined) return resolveProviderViaRouter(iii, PROVIDER_ID);
  if (resolveCache?.key === key) return resolveCache.resolved;
  const resolved = await resolveProviderViaRouter(iii, PROVIDER_ID);
  resolveCache = { key, resolved };
  return resolved;
}

/** Drop the cached resolution so the next stream re-resolves (called on a 401). */
export function invalidateProviderResolveCache(): void {
  resolveCache = null;
}

/** Test seam: clear the cache between cases. */
export function _resetProviderResolveCacheForTests(): void {
  resolveCache = null;
}

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
  preResolved?: Model,
  resolutionKey?: string | number,
  maxOutputOverride?: number,
): Promise<AnthropicConfig> {
  const resolved = await resolveProviderForTurn(iii, resolutionKey);
  if (!resolved.credential) {
    throw new Error(
      'router::provider::resolve returned no credential for provider `anthropic` ' +
        '(set an api key in the llm-router configuration or ANTHROPIC_API_KEY)',
    );
  }
  const apiUrl = resolved.api_url ?? worker.default_api_url;
  const catalog = preResolved ?? (await getCatalogModel(iii, PROVIDER_ID, model));
  // The router's resolved budget (when threaded) is authoritative; it still
  // runs through the model-ceiling clamp as the override, never raised.
  const maxTokens = clampOutputTokens({
    modelMaxOutput: catalog?.max_output_tokens,
    userOverride: maxOutputOverride ?? resolved.max_tokens,
    workerDefault: worker.default_max_tokens,
  });
  const cfg = configWithCredential(model, resolved.credential, maxTokens, apiUrl);
  return catalog ? { ...cfg, catalog } : cfg;
}
