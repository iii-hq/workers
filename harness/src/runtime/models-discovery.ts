/**
 * Shared helpers for cloud-provider model discovery. Cloud providers
 * (anthropic, openai, kimi) hit their upstream `/v1/models` endpoint, map
 * the result onto the catalog `Model` shape with sane per-provider defaults,
 * and register each into the router catalog via `router::models::reconcile`.
 *
 * This mirrors the local-provider discovery in
 * `provider-lmstudio/discover.ts`, extended to remote APIs. The console
 * model dropdown reads the cached result via `router::models::list`.
 */

import type { Model } from '../models-catalog/types.js';
import type { ISdk } from './iii.js';
import type { ModelsDevModel } from './modelsdev.js';
import { logger } from './otel.js';
import { routerRegistrationToken } from './provider-resolve.js';

const DISCOVERY_TIMEOUT_MS = 8_000;
const REGISTER_TIMEOUT_MS = 5_000;

/**
 * Derive a sibling endpoint URL from a chat/messages URL by swapping the
 * trailing path segment, e.g. `.../v1/chat/completions` ->`.../v1/models`
 * or `.../v1/messages` -> `.../v1/models`. Falls back to appending
 * `/models` to the origin when the input doesn't match a known suffix.
 */
export function deriveModelsUrl(apiUrl: string): string {
  const swapped = apiUrl.replace(/\/(chat\/completions|messages|completions)\/?$/, '/models');
  if (swapped !== apiUrl) return swapped;
  try {
    const u = new URL(apiUrl);
    return `${u.protocol}//${u.host}/v1/models`;
  } catch {
    return `${apiUrl.replace(/\/+$/, '')}/models`;
  }
}

async function fetchWithTimeout(
  url: string,
  init: RequestInit,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

export type ModelsFetchResult =
  | { kind: 'ok'; json: unknown }
  | { kind: 'auth_error'; status: number }
  | { kind: 'transient_error'; status?: number };

function isAuthStatus(status: number): boolean {
  return status === 401 || status === 403;
}

/**
 * GET `url` for cloud provider model listing. Distinguishes invalid credentials
 * (401/403) from transient failures so discovery can prune vs keep the catalog.
 * Never throws across the bus boundary.
 */
export async function fetchModelsForDiscovery(
  url: string,
  headers: Record<string, string>,
): Promise<ModelsFetchResult> {
  let resp: Response;
  try {
    resp = await fetchWithTimeout(url, { method: 'GET', headers }, DISCOVERY_TIMEOUT_MS);
  } catch (err) {
    logger.warn('model discovery: fetch failed', { url, err: String(err) });
    return { kind: 'transient_error' };
  }
  if (!resp.ok) {
    if (isAuthStatus(resp.status)) {
      logger.info('model discovery: auth rejected', { url, status: resp.status });
      return { kind: 'auth_error', status: resp.status };
    }
    logger.warn('model discovery: non-2xx response', { url, status: resp.status });
    return { kind: 'transient_error', status: resp.status };
  }
  try {
    return { kind: 'ok', json: await resp.json() };
  } catch (err) {
    logger.warn('model discovery: invalid JSON', { url, err: String(err) });
    return { kind: 'transient_error' };
  }
}

/**
 * GET `url` and return the parsed JSON, or `null` on any error / non-2xx.
 * Discovery is best-effort and must never throw across the bus boundary.
 */
export async function fetchModelsJson(
  url: string,
  headers: Record<string, string>,
): Promise<unknown | null> {
  const result = await fetchModelsForDiscovery(url, headers);
  return result.kind === 'ok' ? result.json : null;
}

export type ModelStub = {
  id: string;
  display_name?: string;
};

/**
 * Build a catalog `Model` for a discovered id. Upstream `/v1/models`
 * endpoints expose little metadata, so per-model limits and capability
 * flags come from models.dev when available (`modelsDev`), with a
 * per-provider default context window and conservative flags as fallback.
 */
export function enrichModel(opts: {
  provider: string;
  api: string;
  stub: ModelStub;
  defaultContextWindow: number;
  modelsDev?: ModelsDevModel;
}): Model {
  const md = opts.modelsDev;
  const model: Model = {
    id: opts.stub.id,
    provider: opts.provider,
    api: opts.api,
    display_name: opts.stub.display_name ?? opts.stub.id,
    context_window: md?.limit?.context ?? opts.defaultContextWindow,
    max_output_tokens: md?.limit?.output ?? 8_192,
    supports_tools: md?.tool_call ?? true,
    transports: ['sse'],
  };
  if (md?.reasoning !== undefined) model.supports_thinking = md.reasoning;
  return model;
}

/**
 * Replace the provider's catalog slice with `models` in one token-gated
 * `router::models::reconcile` call (single state write). Best-effort:
 * failures (including a missing registration token while the router is
 * still coming up) are logged and swallowed so discovery never throws
 * across the bus boundary.
 */
export async function reconcileModels(
  iii: ISdk,
  provider: string,
  models: readonly Model[],
): Promise<string[]> {
  try {
    const token = await routerRegistrationToken(provider);
    await iii.trigger<unknown, { provider?: string; count?: number }>({
      function_id: 'router::models::reconcile',
      payload: { provider, token, models: [...models] },
      timeoutMs: REGISTER_TIMEOUT_MS,
    });
    return models.map((m) => m.id);
  } catch (err) {
    logger.warn('model discovery: reconcile failed', { provider, err: String(err) });
    return [];
  }
}
