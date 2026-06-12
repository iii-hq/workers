/**
 * Per-request output-token resolution.
 *
 * The default request budget is `min(model.max_output_tokens, OUTPUT_TOKEN_MAX)`
 * so high-output models (Claude 64k/128k, GPT-5 272k) don't burn latency/cost
 * on every turn. An explicit registry override always wins, clamped only to the
 * model's hard ceiling (never raised) to avoid upstream 400s.
 */

import type { Model } from '../types/model.js';
import type { ISdk } from './iii.js';
import { logger } from './otel.js';

export const OUTPUT_TOKEN_MAX = 32_000;
export const OUTPUT_TOKEN_MAX_ENV = 'HARNESS_OUTPUT_TOKEN_MAX';

const MODELS_GET_TIMEOUT_MS = 5_000;

let cachedCap: number | null = null;

/** Cap from `HARNESS_OUTPUT_TOKEN_MAX` (strict positive integer) or 32_000. Cached per process. */
export function outputTokenCap(): number {
  if (cachedCap !== null) return cachedCap;
  const v = process.env[OUTPUT_TOKEN_MAX_ENV];
  if (!v) {
    cachedCap = OUTPUT_TOKEN_MAX;
    return cachedCap;
  }
  // Strict digits-only parse: parseInt would silently truncate values like
  // "1e9" to 1, turning a fat-fingered env var into a 1-token output cap.
  const n = /^\d+$/.test(v.trim()) ? Number.parseInt(v.trim(), 10) : Number.NaN;
  cachedCap = Number.isFinite(n) && n > 0 ? n : OUTPUT_TOKEN_MAX;
  return cachedCap;
}

/** Test hook: clear the cached env cap. */
export function _resetOutputTokenCapForTests(): void {
  cachedCap = null;
}

export type ClampArgs = {
  /** Catalog `max_output_tokens` for the model (undefined/0 = unknown). */
  modelMaxOutput: number | null | undefined;
  /** Explicit registry override (`router::provider::resolve` max_tokens). */
  userOverride: number | null;
  /** Provider worker default (`default_max_tokens`, 8192). */
  workerDefault: number;
  /** Defaults to `outputTokenCap()`. */
  cap?: number;
};

/**
 * Resolve the per-request max output tokens.
 *
 * Precedence:
 * 1. `userOverride > 0` — wins; clamped down to the model ceiling when known
 *    (deliberate choice is honored, so the 32k cap does NOT apply here).
 * 2. `min(modelMaxOutput, cap)` — the per-model default.
 * 3. `workerDefault` — unknown model, preserves pre-clamp behavior.
 */
export function clampOutputTokens(args: ClampArgs): number {
  const cap = args.cap ?? outputTokenCap();
  const modelMax =
    typeof args.modelMaxOutput === 'number' && args.modelMaxOutput > 0
      ? args.modelMaxOutput
      : undefined;

  // Floored: token counts must be integers upstream.
  if (typeof args.userOverride === 'number' && args.userOverride > 0) {
    return Math.floor(
      modelMax !== undefined ? Math.min(args.userOverride, modelMax) : args.userOverride,
    );
  }
  if (modelMax !== undefined) return Math.floor(Math.min(modelMax, cap));
  return Math.floor(args.workerDefault);
}

/**
 * Fetch the catalog entry for `(provider, model)` via `router::models::get`
 * (payload key is `id`, the result is wrapped as `{ model }`, null on miss).
 * Best-effort: returns null on miss, timeout, or any bus error — callers fall
 * back to worker defaults so an empty catalog never breaks a request.
 */
export async function getCatalogModel(
  iii: ISdk,
  provider: string,
  modelId: string,
): Promise<Model | null> {
  try {
    const entry = await iii.trigger<unknown, { model?: Model | null } | null>({
      function_id: 'router::models::get',
      payload: { provider, id: modelId },
      timeoutMs: MODELS_GET_TIMEOUT_MS,
    });
    return entry?.model ?? null;
  } catch (err) {
    logger.debug('output-tokens: router::models::get failed', {
      provider,
      model: modelId,
      err: String(err),
    });
    return null;
  }
}
