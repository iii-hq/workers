import { logger } from '../runtime/otel.js';
import type { Credential } from '../auth-credentials/types.js';
import type { ISdk } from '../runtime/iii.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

// llama-server is local-first by default. Unlike LM Studio, llama.cpp has
// no documented "default" bearer string — the server either enforces a
// shared `--api-key` (in which case the caller must match it) or accepts
// any/no token. We therefore:
//   - use LLAMACPP_API_KEY if present (set when the user started
//     llama-server with `--api-key …`)
//   - on loopback without a key, omit Authorization entirely
//   - on non-loopback without a key, omit AND warn (don't ship a
//     synthetic bearer to an arbitrary host)
const CREDENTIAL_PROVIDER_SLUG = 'llamacpp';

function extractKey(cred: Credential | null): string {
  if (!cred) return '';
  return cred.type === 'api_key' ? cred.key : cred.access_token;
}

/**
 * `true` when `url` resolves to a localhost / loopback target. Used to
 * decide whether to warn when no credential is configured.
 */
export function isLoopbackUrl(url: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  const host = parsed.hostname.toLowerCase();
  if (host === 'localhost') return true;
  if (host === '127.0.0.1' || host === '::1' || host === '[::1]') return true;
  if (host.endsWith('.localhost')) return true;
  if (/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(host)) return true;
  return false;
}

export async function fetchCredential(iii: ISdk): Promise<Credential | null> {
  try {
    const cred = await iii.trigger<unknown, Credential | null>({
      function_id: 'auth::get_token',
      payload: { provider: CREDENTIAL_PROVIDER_SLUG },
      timeoutMs: 5_000,
    });
    if (!cred || typeof cred !== 'object' || !('type' in cred)) return null;
    return cred;
  } catch (err) {
    logger.warn('llamacpp.auth: fetchCredential failed; falling back to no-credential', {
      code: 'llamacpp_auth_fetch_failed',
      err: String(err),
    });
    return null;
  }
}

/**
 * Decide which API key (if any) to send for `url`.
 *
 * - Explicit credential present → use it.
 * - Otherwise loopback → null (omit Authorization). llama-server without
 *   `--api-key` accepts unauthenticated requests; we don't manufacture
 *   a synthetic token.
 * - Otherwise (non-loopback, no explicit credential) → null AND a
 *   warning log so operators see they're hitting a remote without
 *   credentials.
 */
export function selectAuthKey(
  cred: Credential | null,
  url: string,
): string | null {
  const key = extractKey(cred);
  if (key.length > 0) return key;
  if (isLoopbackUrl(url)) return null;
  logger.warn(
    'llamacpp.auth: no credential configured AND LLAMACPP_BASE_URL is non-loopback; omitting Authorization header',
    {
      code: 'llamacpp_auth_omitted_nonloopback',
      origin: (() => {
        try {
          const u = new URL(url);
          return `${u.protocol}//${u.host}`;
        } catch {
          return '<unparseable>';
        }
      })(),
    },
  );
  return null;
}

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<ChatCompletionsConfig> {
  const cred = await fetchCredential(iii);
  // Pass the credential through verbatim so configFromCredential can
  // populate api_key (empty string when no credential). Header emission
  // is gated separately in buildAuthHeaders/the stream layer.
  return configFromCredential(
    worker.default_api_url,
    'llamacpp',
    model,
    extractKey(cred).length > 0 ? cred : null,
    worker.default_max_tokens,
  );
}

/**
 * Build the HTTP headers used for any llama-server REST call (chat
 * completions AND `/v1/models` discovery). Shared so the auth dance —
 * credential lookup, loopback-vs-remote decision — lives in exactly
 * one place. Authorization is omitted when no credential applies.
 */
export async function buildAuthHeaders(
  iii: ISdk,
  url: string,
): Promise<Record<string, string>> {
  const cred = await fetchCredential(iii);
  const token = selectAuthKey(cred, url);
  const base: Record<string, string> = {
    'content-type': 'application/json',
  };
  if (token !== null) {
    base.Authorization = `Bearer ${token}`;
  }
  return base;
}
