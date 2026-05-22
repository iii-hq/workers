/**
 * llama-server model discovery — hits OpenAI-compatible `GET /v1/models`
 * and registers the (single) currently-loaded model into the iii models
 * catalog so the picker shows the real id (e.g. `Meta-Llama-3.1-8B`)
 * instead of just the `llamacpp-local` placeholder.
 *
 * Unlike provider-lmstudio:
 *   - llama-server runs exactly ONE model at process startup (set via
 *     `-m model.gguf`), so the endpoint returns at most one entry. There
 *     is no concept of "downloaded but not loaded".
 *   - There is no native v0 endpoint exposing context length / arch.
 *     Discovery only learns the model id; everything else falls back to
 *     the embedded catalog placeholder (`llamacpp-local`).
 *
 * Best-effort: failures (server offline, malformed JSON, register RPC
 * errors) are logged and swallowed — the worker still boots, and the
 * embedded placeholder remains usable as a fallback.
 */

import type { Model } from '../models-catalog/types.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

const DISCOVERY_TIMEOUT_MS = 5_000;
const REGISTER_TIMEOUT_MS = 5_000;

const DEFAULT_CONTEXT_WINDOW = 32_768;
const DEFAULT_MAX_OUTPUT_TOKENS = 8_192;

/** OpenAI-compatible `/v1/models` per-entry shape, parsed defensively. */
type OpenAiModel = {
  id?: unknown;
  object?: unknown;
};

type OpenAiModelsResponse = {
  data?: unknown;
};

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
 * Derive the sibling `/v1/models` endpoint from a chat-completions URL
 * by trimming the trailing `/chat/completions`. Works for the default
 * `…/v1/chat/completions` and for any custom-path proxy that follows
 * the same convention.
 */
export function modelsUrl(chatUrl: string): string {
  const trimmed = chatUrl.replace(/\/chat\/completions\/?$/, '');
  return `${trimmed}/models`;
}

/**
 * Fetch and parse the loaded model list. Returns an empty array on any
 * failure (caller decides whether to surface it).
 */
async function fetchModelIds(
  modelsEndpoint: string,
  headers: Record<string, string>,
): Promise<string[]> {
  let resp: Response;
  try {
    resp = await fetchWithTimeout(modelsEndpoint, { method: 'GET', headers }, DISCOVERY_TIMEOUT_MS);
  } catch (err) {
    logger.warn('llamacpp discovery: fetch failed', {
      url: modelsEndpoint,
      err: String(err),
    });
    return [];
  }
  if (!resp.ok) {
    logger.warn('llamacpp discovery: non-2xx response', {
      url: modelsEndpoint,
      status: resp.status,
    });
    return [];
  }
  let parsed: OpenAiModelsResponse;
  try {
    parsed = (await resp.json()) as OpenAiModelsResponse;
  } catch (err) {
    logger.warn('llamacpp discovery: malformed JSON response', {
      url: modelsEndpoint,
      err: String(err),
    });
    return [];
  }
  if (!Array.isArray(parsed.data)) return [];
  const ids: string[] = [];
  for (const entry of parsed.data) {
    if (!entry || typeof entry !== 'object') continue;
    const e = entry as OpenAiModel;
    if (typeof e.id === 'string' && e.id.length > 0) {
      ids.push(e.id);
    }
  }
  return ids;
}

/**
 * Build a placeholder-shaped Model row for each id returned by
 * /v1/models. Field defaults match the `llamacpp-local` catalog
 * placeholder so capability gating (supports_tools etc.) works for
 * arbitrary user-loaded models.
 */
function toCatalogModel(id: string): Model {
  return {
    id,
    provider: 'llamacpp',
    api: 'openai-completions',
    display_name: id,
    context_window: DEFAULT_CONTEXT_WINDOW,
    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
    supports_thinking: false,
    supports_xhigh: false,
    supports_tools: true,
    supports_vision: false,
    supports_cache: false,
    transports: ['sse'],
  };
}

export async function discoverLoadedModel(
  chatUrl: string,
  headers: Record<string, string>,
): Promise<Model[]> {
  const ids = await fetchModelIds(modelsUrl(chatUrl), headers);
  return ids.map(toCatalogModel);
}

/**
 * Register the discovered model(s) via `models::register`. Best-effort
 * per-model — failures are logged and skipped.
 *
 * Parallel via Promise.allSettled — same rationale as
 * provider-lmstudio: serializing register calls bottlenecks startup.
 */
export async function registerDiscovered(iii: ISdk, models: readonly Model[]): Promise<string[]> {
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
    if (r.status === 'fulfilled') {
      registered.push(r.value);
    } else {
      logger.warn('llamacpp discovery: register failed', {
        id: models[i]?.id,
        err: String(r.reason),
      });
    }
  });
  return registered;
}

/**
 * One-shot: discover the loaded llama-server model and register it
 * into the iii models catalog. Used at worker startup (fire-and-forget)
 * and by `provider::llamacpp::refresh_models` on demand.
 */
export async function discoverAndRegister(
  iii: ISdk,
  chatUrl: string,
  headers: Record<string, string>,
): Promise<string[]> {
  const models = await discoverLoadedModel(chatUrl, headers);
  if (models.length === 0) {
    logger.info('llamacpp discovery: no loaded model found', {});
    return [];
  }
  const registered = await registerDiscovered(iii, models);
  logger.info('llamacpp discovery: registered models', { count: registered.length });
  logger.debug('llamacpp discovery: registered model ids', { ids: registered });
  return registered;
}
