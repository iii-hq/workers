import type { Credential } from '../auth-credentials/types.js';
import type { ISdk } from '../runtime/iii.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

// LM Studio is local-first: by default the localhost REST server runs without
// authentication and ignores the Authorization header. We still go through
// `auth::get_token` so users who run an authenticated LM Studio deployment
// can opt-in via `LMSTUDIO_API_KEY`, but when the credential is missing or
// empty we fall back to the literal string `"lm-studio"` (LM Studio's own
// convention — see https://lmstudio.ai/docs/local-server). The header is
// always sent so corporate proxies that require *some* token also work.
const CREDENTIAL_PROVIDER_SLUG = 'lmstudio';
const FALLBACK_API_KEY = 'lm-studio';

function extractKey(cred: Credential | null): string {
  if (!cred) return '';
  return cred.type === 'api_key' ? cred.key : cred.access_token;
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
  } catch {
    // auth-credentials worker not reachable, no credential entry, etc.
    // For LM Studio this is a normal localhost-no-auth setup; don't throw.
    return null;
  }
}

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<ChatCompletionsConfig> {
  const cred = await fetchCredential(iii);
  const key = extractKey(cred);
  const effective: Credential =
    key.length > 0
      ? (cred as Credential)
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
 * Build the HTTP headers used for any LM Studio REST call (chat completions
 * AND `/api/v0/models` discovery). Shared so the auth dance — credential
 * lookup, fall back to the literal `"lm-studio"` key — lives in exactly
 * one place.
 */
export async function buildAuthHeaders(
  iii: ISdk,
): Promise<Record<string, string>> {
  const cred = await fetchCredential(iii);
  const key = extractKey(cred);
  const token = key.length > 0 ? key : FALLBACK_API_KEY;
  return {
    'content-type': 'application/json',
    Authorization: `Bearer ${token}`,
  };
}
