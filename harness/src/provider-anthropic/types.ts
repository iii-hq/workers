/**
 * Anthropic provider config + auth-mode union. Mirrors
 * `provider-anthropic/src/lib.rs::{AnthropicConfig, AuthMode}`.
 */

import type { Credential } from '../runtime/provider-resolve.js';
import { DEFAULT_API_URL } from './config.js';

export type AuthMode = 'api_key' | 'oauth_bearer';

export type AnthropicConfig = {
  credential_value: string;
  model: string;
  max_tokens: number;
  api_url: string;
  auth_mode: AuthMode;
};

export function configWithCredential(
  model: string,
  cred: Credential,
  max_tokens = 4096,
  api_url = DEFAULT_API_URL,
): AnthropicConfig {
  if (cred.type === 'api_key') {
    return { credential_value: cred.key, model, max_tokens, api_url, auth_mode: 'api_key' };
  }
  return {
    credential_value: cred.access_token,
    model,
    max_tokens,
    api_url,
    auth_mode: 'oauth_bearer',
  };
}

/** Pure helper: produces the `[headerName, headerValue]` pair for a config. */
export function authHeaderFor(cfg: AnthropicConfig): [string, string] {
  if (cfg.auth_mode === 'api_key') return ['x-api-key', cfg.credential_value];
  return ['authorization', `Bearer ${cfg.credential_value}`];
}
