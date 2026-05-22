/**
 * HTTP helpers for LM Studio's native v1 model management endpoints:
 *   - `POST /api/v1/models/load`   — block until a model is in memory
 *   - `POST /api/v1/models/unload` — release a loaded instance
 *
 * These are LM Studio 0.4+ only (the v1 native API). They're not exposed
 * on the legacy `/v1/*` OpenAI-compatible path or the `/api/v0/*` native
 * path. Callers that hit older LM Studio versions will get a 404; the
 * stream handler tolerates that (JIT can still kick in, or the chat call
 * will surface a clear error).
 */

import { logger } from '../runtime/otel.js';

/** Load can take a while for large models on consumer hardware. */
const LOAD_TIMEOUT_MS = 120_000;
/** Unload is fast — just frees memory. */
const UNLOAD_TIMEOUT_MS = 10_000;

/** All optional knobs the load endpoint accepts. */
export type LoadOptions = {
  context_length?: number;
  eval_batch_size?: number;
  flash_attention?: boolean;
  num_experts?: number;
  offload_kv_cache_to_gpu?: boolean;
  /** When true, the response includes the resolved `load_config`. */
  echo_load_config?: boolean;
};

/** Shape of a successful load response. */
export type LoadResult = {
  type: 'llm' | 'embedding' | string;
  instance_id: string;
  load_time_seconds: number;
  status: 'loaded' | string;
  load_config?: Record<string, unknown>;
};

/** Shape of a successful unload response. */
export type UnloadResult = {
  instance_id: string;
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
 * Derive `/api/v1/models/load` from any chat-completions URL we accept.
 * Supports the OpenAI-compat `/v1/chat/completions`, the native v0
 * `/api/v0/chat/completions`, and any future `/api/vN/chat/completions`.
 */
export function nativeLoadUrl(chatUrl: string): string {
  const trimmed = chatUrl.replace(/\/(v1|api\/v\d+)\/chat\/completions\/?$/, '');
  return `${trimmed}/api/v1/models/load`;
}

export function nativeUnloadUrl(chatUrl: string): string {
  const trimmed = chatUrl.replace(/\/(v1|api\/v\d+)\/chat\/completions\/?$/, '');
  return `${trimmed}/api/v1/models/unload`;
}

/**
 * Load a model into memory. Blocks until LM Studio reports `status: "loaded"`
 * or the timeout fires. Throws on transport error, non-2xx status, or
 * timeout — callers decide whether to swallow or surface.
 */
export async function loadModel(
  chatUrl: string,
  headers: Record<string, string>,
  model: string,
  options: LoadOptions = {},
): Promise<LoadResult> {
  const url = nativeLoadUrl(chatUrl);
  const body: Record<string, unknown> = { model, ...options };
  let resp: Response;
  try {
    resp = await fetchWithTimeout(
      url,
      { method: 'POST', headers, body: JSON.stringify(body) },
      LOAD_TIMEOUT_MS,
    );
  } catch (err) {
    if (err instanceof Error && err.name === 'AbortError') {
      throw new Error(
        `lmstudio load timed out after ${LOAD_TIMEOUT_MS / 1000}s for model "${model}"`,
      );
    }
    throw err;
  }
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    // Surface common-causes guidance alongside the raw 500 body so the
    // operator has a checklist instead of just "Failed to load model.".
    // LM Studio's own error message is intentionally terse; the real
    // diagnosis is almost always one of these four:
    const hint =
      resp.status === 404
        ? `(404: native /api/v1/models/load is LM Studio 0.4+ only — older builds will need a manual load in the GUI)`
        : `(common causes: model not downloaded in LM Studio; chat template missing or incompatible; insufficient VRAM/RAM; quantization mismatch — open LM Studio's "Developer" panel to see the underlying load error)`;
    throw new Error(
      `lmstudio load failed (${resp.status}) for model "${model}": ${text || 'no body'} ${hint}`,
    );
  }
  const result = (await resp.json()) as LoadResult;
  logger.info('lmstudio: model loaded', {
    model,
    instance_id: result.instance_id,
    load_time_seconds: result.load_time_seconds,
  });
  return result;
}

/**
 * Unload a previously-loaded model instance.
 */
export async function unloadModel(
  chatUrl: string,
  headers: Record<string, string>,
  instance_id: string,
): Promise<UnloadResult> {
  const url = nativeUnloadUrl(chatUrl);
  let resp: Response;
  try {
    resp = await fetchWithTimeout(
      url,
      {
        method: 'POST',
        headers,
        body: JSON.stringify({ instance_id }),
      },
      UNLOAD_TIMEOUT_MS,
    );
  } catch (err) {
    if (err instanceof Error && err.name === 'AbortError') {
      throw new Error(
        `lmstudio unload timed out after ${UNLOAD_TIMEOUT_MS / 1000}s for "${instance_id}"`,
      );
    }
    throw err;
  }
  if (!resp.ok) {
    const text = await resp.text().catch(() => '');
    throw new Error(
      `lmstudio unload failed (${resp.status}): ${text || 'no body'} (instance_id="${instance_id}")`,
    );
  }
  const result = (await resp.json()) as UnloadResult;
  logger.info('lmstudio: model unloaded', { instance_id: result.instance_id });
  return result;
}
