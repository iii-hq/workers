/**
 * LM Studio model discovery — hits the native `GET /api/v0/models` endpoint
 * and registers each loaded LLM into the iii models catalog so the
 * picker/dropdown shows them by their real IDs (e.g. `qwen/qwen3-4b-2507`)
 * instead of just the `lmstudio-local` placeholder.
 *
 * Why the native v0 endpoint and not OpenAI-compatible `/v1/models`:
 *   - v0 returns `state` (loaded / not-loaded / loading) so we can filter
 *     to only models that are actually serving right now.
 *   - v0 returns `type` (llm / vlm / embeddings) so we skip non-chat models.
 *   - v0 returns `loaded_context_length` / `max_context_length` so the
 *     catalog row carries the real context window.
 *
 * Best-effort: all failures (LM Studio offline, malformed JSON, register
 * RPC errors) are logged and swallowed — the worker still boots, and the
 * embedded `lmstudio-local` placeholder remains usable as a fallback.
 */

import type { Model } from '../models-catalog/types.js';
import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';

/** Timeout for the discovery HTTP call. Short — LM Studio is local. */
const DISCOVERY_TIMEOUT_MS = 5_000;

/** Timeout for each `models::register` bus call. */
const REGISTER_TIMEOUT_MS = 5_000;

/** Defaults for fields LM Studio doesn't expose directly. */
const DEFAULT_CONTEXT_WINDOW = 32_768;
const DEFAULT_MAX_OUTPUT_TOKENS = 8_192;

/**
 * LM Studio's `/api/v0/models` per-entry shape. Parsed defensively so the
 * code keeps working if LM Studio adds or renames fields between versions.
 */
type LmstudioModel = {
  id?: unknown;
  type?: unknown;
  state?: unknown;
  arch?: unknown;
  quantization?: unknown;
  publisher?: unknown;
  max_context_length?: unknown;
  loaded_context_length?: unknown;
};

type LmstudioModelsResponse = {
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
 * Derive the native `/api/v0/models` endpoint from a chat-completions URL.
 *
 * Handles both URL forms the worker can be configured with:
 *   - OpenAI-compatible: `http://host:port/v1/chat/completions`
 *   - LM Studio native:  `http://host:port/api/v0/chat/completions`
 *
 * Anything else (custom proxy path) falls through to `<base>/api/v0/models`
 * by appending — the caller's URL is preserved.
 */
export function nativeModelsUrl(chatUrl: string): string {
  const trimmed = chatUrl.replace(/\/(v1|api\/v\d+)\/chat\/completions\/?$/, '');
  return `${trimmed}/api/v0/models`;
}

function asString(v: unknown): string | null {
  return typeof v === 'string' && v.length > 0 ? v : null;
}

function asPositiveNumber(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) && v > 0 ? v : null;
}

/**
 * Convert one LM Studio model entry into a catalog `Model`.
 *
 * Returns `null` for entries that aren't usable LLMs (no id, embeddings,
 * or any non-llm/non-vlm type LM Studio surfaces).
 */
export function toCatalogModel(m: LmstudioModel): Model | null {
  const id = asString(m.id);
  if (!id) return null;
  const type = asString(m.type);
  // `llm` = text models; `vlm` = vision-language; anything else (embedding,
  // audio, ...) the chat orchestrator doesn't route to.
  if (type !== null && type !== 'llm' && type !== 'vlm') return null;
  const context_window =
    asPositiveNumber(m.loaded_context_length) ??
    asPositiveNumber(m.max_context_length) ??
    DEFAULT_CONTEXT_WINDOW;
  return {
    id,
    provider: 'lmstudio',
    api: 'openai-completions',
    display_name: id,
    context_window,
    max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
    supports_thinking: false,
    supports_xhigh: false,
    supports_tools: true,
    supports_vision: type === 'vlm',
    supports_cache: false,
    transports: ['sse'],
  };
}

/**
 * Raw fetch of `/api/v0/models`. Returns the parsed `data` array verbatim
 * (no filtering, no catalog mapping) so callers can pick which subset they
 * need: loaded only, downloaded, by id, etc. Returns `[]` on any error.
 */
async function fetchRawModels(
  chatUrl: string,
  headers: Record<string, string>,
): Promise<LmstudioModel[]> {
  const url = nativeModelsUrl(chatUrl);
  let resp: Response;
  try {
    resp = await fetchWithTimeout(
      url,
      { method: 'GET', headers },
      DISCOVERY_TIMEOUT_MS,
    );
  } catch (err) {
    logger.warn('lmstudio discovery: fetch failed', { url, err: String(err) });
    return [];
  }
  if (!resp.ok) {
    logger.warn('lmstudio discovery: non-2xx response', {
      url,
      status: resp.status,
    });
    return [];
  }
  let parsed: LmstudioModelsResponse;
  try {
    parsed = (await resp.json()) as LmstudioModelsResponse;
  } catch (err) {
    logger.warn('lmstudio discovery: invalid JSON', { url, err: String(err) });
    return [];
  }
  return Array.isArray(parsed.data) ? (parsed.data as LmstudioModel[]) : [];
}

/**
 * Fetch `/api/v0/models` and return the catalog `Model` for each LLM/VLM
 * that is *currently loaded*. Used by the chat stream handler when the
 * picker sent the `lmstudio-local` placeholder — resolves it to whatever
 * is loaded right now.
 *
 * Returns `[]` on any error or when no LLMs are currently loaded.
 */
export async function discoverLoadedModels(
  chatUrl: string,
  headers: Record<string, string>,
): Promise<Model[]> {
  const entries = await fetchRawModels(chatUrl, headers);
  return entries
    .filter((m) => asString(m.state) === 'loaded')
    .map(toCatalogModel)
    .filter((m): m is Model => m !== null);
}

/**
 * Fetch `/api/v0/models` and return the catalog `Model` for *every*
 * downloaded LLM/VLM, loaded or not. Used by startup discovery so the
 * picker shows all the user's available models — they can pick one
 * that's not loaded yet and the stream handler will auto-load it.
 */
export async function discoverAllDownloadedModels(
  chatUrl: string,
  headers: Record<string, string>,
): Promise<Model[]> {
  const entries = await fetchRawModels(chatUrl, headers);
  return entries.map(toCatalogModel).filter((m): m is Model => m !== null);
}

/**
 * Fetch `/api/v0/models` and return a `Set` of model IDs that are
 * currently in `state == "loaded"`. Cheap helper used by the stream
 * handler to decide whether it needs to call the load endpoint before
 * sending the chat request.
 */
export async function discoverLoadedIds(
  chatUrl: string,
  headers: Record<string, string>,
): Promise<Set<string>> {
  const entries = await fetchRawModels(chatUrl, headers);
  const ids = new Set<string>();
  for (const m of entries) {
    if (asString(m.state) !== 'loaded') continue;
    const id = asString(m.id);
    if (id) ids.add(id);
  }
  return ids;
}

/**
 * Register each discovered model via the `models::register` bus function.
 * Best-effort per-model: failures are logged and skipped, the rest are
 * still registered.
 *
 * Fan out in parallel via Promise.allSettled — the bus has no ordering
 * requirement between registers, and serializing them stretched
 * startup-blocking time to N × REGISTER_TIMEOUT_MS for users with
 * many downloaded models. Pre-fix: 30 models × 5s timeout could leave
 * the worker un-discoverable for 150s. Now: ~5s worst case regardless
 * of N.
 */
export async function registerDiscovered(
  iii: ISdk,
  models: readonly Model[],
): Promise<string[]> {
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
      logger.warn('lmstudio discovery: register failed', {
        id: models[i]?.id,
        err: String(r.reason),
      });
    }
  });
  return registered;
}

/**
 * One-shot: discover every downloaded LM Studio LLM/VLM and register each
 * into the iii models catalog. Used at worker startup (fire-and-forget)
 * and by the `provider::lmstudio::refresh_models` bus function on demand.
 *
 * We register *all* downloaded models, not just the loaded ones, so the
 * picker shows everything the user could pick. The stream handler
 * auto-loads not-yet-loaded models on first use (see stream.ts).
 */
export async function discoverAndRegister(
  iii: ISdk,
  chatUrl: string,
  headers: Record<string, string>,
): Promise<string[]> {
  const models = await discoverAllDownloadedModels(chatUrl, headers);
  if (models.length === 0) {
    logger.info('lmstudio discovery: no downloaded LLMs found', {});
    return [];
  }
  const registered = await registerDiscovered(iii, models);
  // Log count at INFO; full id list at DEBUG so model identifiers
  // (which may carry fine-tune / org context) don't leak into
  // shared observability tooling on the cheaper tier.
  logger.info('lmstudio discovery: registered models', {
    count: registered.length,
  });
  logger.debug('lmstudio discovery: registered model ids', {
    ids: registered,
  });
  return registered;
}
