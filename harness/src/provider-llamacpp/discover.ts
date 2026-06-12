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
 *   - There is no v0-style endpoint that surfaces per-model context
 *     length in `/v1/models`, but the server-wide `GET /props` endpoint
 *     does expose `n_ctx` (the value passed via `-c` / `--ctx-size` at
 *     startup), shared across all slots. We fetch it alongside the
 *     models list and use it to populate `context_window`; otherwise we
 *     fall back to the embedded catalog placeholder (`llamacpp-local`).
 *
 * Best-effort: failures (server offline, malformed JSON, register RPC
 * errors) are logged and swallowed — the worker still boots, and the
 * embedded placeholder remains usable as a fallback.
 */

import type { Model } from '../types/model.js';
import type { ISdk } from '../runtime/iii.js';
import { reconcileModels } from '../runtime/models-discovery.js';
import { logger } from '../runtime/otel.js';
import { PROVIDER_ID } from './auth.js';

const DISCOVERY_TIMEOUT_MS = 5_000;
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
 * Derive the server-wide `/props` endpoint from a chat-completions URL.
 *
 * `/props` lives at the server root (not under `/v1`), so we strip both
 * a `…/chat/completions` suffix and a trailing `/v1` (or any other path
 * segment), then append `/props`.
 *
 * Examples:
 *   http://host:8080/v1/chat/completions → http://host:8080/props
 *   http://host:8080/v1/                 → http://host:8080/props
 *   http://host:8080/custom/path         → http://host:8080/custom/path/props
 */
export function propsUrl(chatUrl: string): string {
  const withoutChat = chatUrl.replace(/\/chat\/completions\/?$/, '');
  const withoutV1 = withoutChat.replace(/\/v1\/?$/, '');
  const trimmed = withoutV1.endsWith('/') ? withoutV1.slice(0, -1) : withoutV1;
  return `${trimmed}/props`;
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
 * `GET /props` response shape (only the fields we read). `n_ctx` is the
 * server-wide context size from `-c` / `--ctx-size` at startup and is
 * shared across slots — applies to whichever single model llama-server
 * has loaded. Older llama-server builds only expose `n_ctx` nested in
 * `default_generation_settings`; newer builds also expose it at the top
 * level. We read both and prefer the top-level field when present.
 */
type LlamaProps = {
  n_ctx?: unknown;
  default_generation_settings?: unknown;
};

function isPositiveInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0;
}

function readNestedNCtx(value: unknown): number | null {
  if (!value || typeof value !== 'object') return null;
  const v = (value as Record<string, unknown>).n_ctx;
  return isPositiveInteger(v) ? v : null;
}

/**
 * Fetch the server-wide context window from `GET /props`. Returns the
 * positive integer `n_ctx`, or `null` on any failure (server offline,
 * non-2xx, malformed JSON, missing/non-positive field). Best-effort —
 * a `null` return causes the caller to fall back to DEFAULT_CONTEXT_WINDOW.
 */
async function fetchPropsContextWindow(
  endpoint: string,
  headers: Record<string, string>,
): Promise<number | null> {
  let resp: Response;
  try {
    resp = await fetchWithTimeout(endpoint, { method: 'GET', headers }, DISCOVERY_TIMEOUT_MS);
  } catch (err) {
    logger.warn('llamacpp discovery: /props fetch failed', {
      url: endpoint,
      err: String(err),
    });
    return null;
  }
  if (!resp.ok) {
    logger.warn('llamacpp discovery: /props non-2xx response', {
      url: endpoint,
      status: resp.status,
    });
    return null;
  }
  let parsed: LlamaProps;
  try {
    parsed = (await resp.json()) as LlamaProps;
  } catch (err) {
    logger.warn('llamacpp discovery: /props malformed JSON response', {
      url: endpoint,
      err: String(err),
    });
    return null;
  }
  if (!parsed || typeof parsed !== 'object') return null;
  if (isPositiveInteger(parsed.n_ctx)) return parsed.n_ctx;
  const nested = readNestedNCtx(parsed.default_generation_settings);
  if (nested !== null) return nested;
  logger.warn('llamacpp discovery: /props response missing usable n_ctx', {
    url: endpoint,
  });
  return null;
}

/**
 * Build a placeholder-shaped Model row for each id returned by
 * /v1/models. Field defaults match the `llamacpp-local` catalog
 * placeholder so capability gating (supports_tools etc.) works for
 * arbitrary user-loaded models. `contextWindow` is the value from
 * `/props.n_ctx` when available; callers pass DEFAULT_CONTEXT_WINDOW
 * as a fallback.
 */
function toCatalogModel(id: string, contextWindow: number): Model {
  return {
    id,
    provider: 'llamacpp',
    api: 'openai-completions',
    display_name: id,
    context_window: contextWindow,
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
  const [ids, ctx] = await Promise.all([
    fetchModelIds(modelsUrl(chatUrl), headers),
    fetchPropsContextWindow(propsUrl(chatUrl), headers),
  ]);
  const contextWindow = ctx ?? DEFAULT_CONTEXT_WINDOW;
  // Visible at INFO so operators can tell from the harness log whether
  // /props reported the real n_ctx or we fell through to the default —
  // distinguishes "served context too small" from "discovery couldn't
  // see n_ctx" without needing to attach a debugger.
  logger.info('llamacpp discovery: resolved context window', {
    context_window: contextWindow,
    source: ctx === null ? 'default_fallback' : 'props_n_ctx',
  });
  return ids.map((id) => toCatalogModel(id, contextWindow));
}

/** Register discovered models in one `models::reconcile` call. */
export async function registerDiscovered(iii: ISdk, models: readonly Model[]): Promise<string[]> {
  if (models.length === 0) return [];
  const provider = models[0]?.provider;
  if (!provider) return [];
  return reconcileModels(iii, provider, models);
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
    // Empty can mean "server offline" or "no model loaded" — we can't tell
    // them apart here, so keep the last-known catalog rather than risk wiping
    // it on a transient blip.
    logger.info('llamacpp discovery: no loaded model found', {});
    return [];
  }
  const registered = await reconcileModels(iii, PROVIDER_ID, models);
  logger.info('llamacpp discovery: registered models', { count: registered.length });
  logger.debug('llamacpp discovery: registered model ids', { ids: registered });
  return registered;
}
