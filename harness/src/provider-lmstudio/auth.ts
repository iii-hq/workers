import { logger } from '../runtime/otel.js';
import type { Credential } from '../auth-credentials/types.js';
import type { ISdk } from '../runtime/iii.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

// LM Studio is local-first: by default the localhost REST server runs without
// authentication and ignores the Authorization header. We still go through
// `auth::get_token` so users who run an authenticated LM Studio deployment
// can opt-in via `LMSTUDIO_API_KEY`, but when the credential is missing or
// empty AND the URL points at loopback we fall back to the literal string
// `"lm-studio"` (LM Studio's own convention — see
// https://lmstudio.ai/docs/local-server). For non-loopback hosts we refuse
// to send a fallback bearer: a misconfigured LMSTUDIO_BASE_URL (e.g. via a
// stale EnvironmentFile, a tunnel host, an ssh-forward) would otherwise leak
// a recognisable provider fingerprint to whoever is on the other end.
const CREDENTIAL_PROVIDER_SLUG = 'lmstudio';
const FALLBACK_API_KEY = 'lm-studio';

function extractKey(cred: Credential | null): string {
  if (!cred) return '';
  return cred.type === 'api_key' ? cred.key : cred.access_token;
}

/**
 * `true` when `url` resolves to a localhost / loopback target. Used to
 * decide whether the fallback bearer is safe to send.
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
  // 127.0.0.0/8 — IPv4 loopback block
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
    // auth-credentials worker not reachable, no credential entry, etc.
    // For LM Studio this is a normal localhost-no-auth setup; don't throw.
    // Log at WARN with a stable code so monitoring can alert on a
    // sustained fallback-key rate (e.g. an attacker DoSing the auth
    // worker to silently downgrade us to the fallback bearer).
    logger.warn('lmstudio.auth: fetchCredential failed; falling back to no-credential', {
      code: 'lmstudio_auth_fetch_failed',
      err: String(err),
    });
    return null;
  }
}

/**
 * Decide which API key (if any) to send for `url`.
 *
 * - Explicit credential present → use it.
 * - Otherwise loopback → send the LM Studio fallback ("lm-studio").
 * - Otherwise (non-loopback, no explicit credential) → null, meaning
 *   omit the Authorization header entirely. Sending a fallback bearer
 *   to an arbitrary host can leak the provider fingerprint to whoever
 *   operates that host.
 */
export function selectAuthKey(cred: Credential | null, url: string): string | null {
  const key = extractKey(cred);
  if (key.length > 0) return key;
  if (isLoopbackUrl(url)) return FALLBACK_API_KEY;
  logger.warn(
    'lmstudio.auth: no credential configured AND LMSTUDIO_BASE_URL is non-loopback; omitting Authorization header',
    {
      code: 'lmstudio_auth_omitted_nonloopback',
      // Don't log the full URL — it might carry tokens in query
      // params or path. Just the origin (scheme + host + port).
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
  const key = selectAuthKey(cred, worker.default_api_url);
  // configFromCredential expects a Credential. When no key is
  // available we still pass a synthetic api_key with the LM Studio
  // fallback string so the downstream interface stays uniform; the
  // calling code (stream.ts / discover.ts) goes through
  // buildAuthHeaders for the actual header emission, where the
  // null-key case omits the Authorization header.
  const effective: Credential =
    key !== null
      ? cred && extractKey(cred).length > 0
        ? (cred as Credential)
        : { type: 'api_key', key }
      : { type: 'api_key', key: FALLBACK_API_KEY };
  return configFromCredential(
    worker.default_api_url,
    'lmstudio',
    model,
    effective,
    worker.default_max_tokens,
  );
}

/**
 * Build the HTTP headers used for any LM Studio REST call (chat
 * completions AND `/api/v0/models` discovery). Shared so the auth
 * dance — credential lookup, loopback-vs-remote decision — lives in
 * exactly one place.
 */
export async function buildAuthHeaders(iii: ISdk, url: string): Promise<Record<string, string>> {
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
