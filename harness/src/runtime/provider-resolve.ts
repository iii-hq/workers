/**
 * Provider-side helpers for the harness provider registry:
 *
 *   - `declareProvider` announces a provider (id, defaults, env var) at
 *     startup so the harness composes it into the dynamic `harness`
 *     configuration schema.
 *   - `resolveProvider` fetches the credential + settings (api_url,
 *     max_tokens) at request time, replacing the old `auth::get_token` +
 *     `provider_config::get` pair with one call.
 *
 * Declaration is best-effort (logged on failure — the provider still serves
 * with its config.yaml defaults and the env var declared via
 * `credential_env_var`).
 *
 * This module also owns the `Credential` shape returned by
 * `harness::provider::resolve` (the resolved api key / oauth token).
 */

import type { ISdk } from './iii.js';
import { logger } from './otel.js';

export type ApiKeyCredential = {
  type: 'api_key';
  key: string;
};

export type OAuthCredential = {
  type: 'oauth';
  access_token: string;
  refresh_token?: string;
  expires_at?: number;
  scopes?: string[];
  provider_extra?: unknown;
};

export type Credential = ApiKeyCredential | OAuthCredential;

export type ProviderDeclaration = {
  id: string;
  display_name?: string;
  /** Env var consulted as a credential fallback when no `api_key` is configured. */
  credential_env_var?: string;
  /** Optional explicit JSON Schema; when omitted the harness derives one from `defaults`. */
  config_schema?: Record<string, unknown>;
  defaults?: { api_url?: string; max_tokens?: number } & Record<string, unknown>;
  supports_model_listing?: boolean;
};

export type ProviderResolveResult = {
  configured: boolean;
  source: 'stored' | 'environment' | 'runtime' | 'fallback' | null;
  credential: Credential | null;
  api_url: string | null;
  max_tokens: number | null;
};

const DECLARE_TIMEOUT_MS = 5_000;
const RESOLVE_TIMEOUT_MS = 5_000;

export async function declareProvider(iii: ISdk, decl: ProviderDeclaration): Promise<void> {
  try {
    await iii.trigger({
      function_id: 'harness::provider::register',
      payload: decl,
      timeoutMs: DECLARE_TIMEOUT_MS,
    });
  } catch (err) {
    logger.warn('provider declare failed', { provider: decl.id, err: String(err) });
  }
}

export async function resolveProvider(iii: ISdk, provider: string): Promise<ProviderResolveResult> {
  const res = await iii.trigger<unknown, ProviderResolveResult>({
    function_id: 'harness::provider::resolve',
    payload: { provider },
    timeoutMs: RESOLVE_TIMEOUT_MS,
  });
  if (!res || typeof res !== 'object') {
    return { configured: false, source: null, credential: null, api_url: null, max_tokens: null };
  }
  return res;
}
