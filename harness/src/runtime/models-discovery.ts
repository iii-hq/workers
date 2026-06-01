/**
 * Shared helpers for cloud-provider model discovery. Cloud providers
 * (anthropic, openai, kimi) hit their upstream `/v1/models` endpoint, map
 * the result onto the catalog `Model` shape with sane per-provider defaults,
 * and register each into the iii models catalog via `models::register`.
 *
 * This mirrors the local-provider discovery in
 * `provider-lmstudio/discover.ts`, extended to remote APIs. The console
 * model dropdown reads the cached result via `models::list`.
 */

import type { Model } from '../models-catalog/types.js';
import type { ISdk } from './iii.js';
import { logger } from './otel.js';

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
    return apiUrl.replace(/\/+$/, '') + '/models';
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

/**
 * GET `url` and return the parsed JSON, or `null` on any error / non-2xx.
 * Discovery is best-effort and must never throw across the bus boundary.
 */
export async function fetchModelsJson(
  url: string,
  headers: Record<string, string>,
): Promise<unknown | null> {
  let resp: Response;
  try {
    resp = await fetchWithTimeout(url, { method: 'GET', headers }, DISCOVERY_TIMEOUT_MS);
  } catch (err) {
    logger.warn('model discovery: fetch failed', { url, err: String(err) });
    return null;
  }
  if (!resp.ok) {
    logger.warn('model discovery: non-2xx response', { url, status: resp.status });
    return null;
  }
  try {
    return await resp.json();
  } catch (err) {
    logger.warn('model discovery: invalid JSON', { url, err: String(err) });
    return null;
  }
}

export type ModelStub = {
  id: string;
  display_name?: string;
};

/**
 * Build a catalog `Model` for a discovered id. Upstream `/v1/models`
 * endpoints expose little metadata, so we fill in a per-provider default
 * context window and conservative capability flags (tools on, the rest
 * left at their defaults).
 */
export function enrichModel(opts: {
  provider: string;
  api: string;
  stub: ModelStub;
  defaultContextWindow: number;
}): Model {
  return {
    id: opts.stub.id,
    provider: opts.provider,
    api: opts.api,
    display_name: opts.stub.display_name ?? opts.stub.id,
    context_window: opts.defaultContextWindow,
    max_output_tokens: 8_192,
    supports_tools: true,
    transports: ['sse'],
  };
}

/** Register each model into the catalog. Best-effort, parallel, never throws. */
export async function registerModels(iii: ISdk, models: readonly Model[]): Promise<string[]> {
  const settled = await Promise.allSettled(
    models.map(async (m) => {
      await iii.trigger({
        function_id: 'models::register',
        payload: m,
        timeoutMs: REGISTER_TIMEOUT_MS,
      });
      return m.id;
    }),
  );
  const registered: string[] = [];
  settled.forEach((r, i) => {
    if (r.status === 'fulfilled') registered.push(r.value);
    else
      logger.warn('model discovery: register failed', {
        id: models[i]?.id,
        err: String(r.reason),
      });
  });
  return registered;
}
