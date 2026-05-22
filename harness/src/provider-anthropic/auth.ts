/**
 * Bridge to `auth::get_token`. The provider asks the auth worker for a
 * resolved Credential, then turns it into an AnthropicConfig.
 */

import type { Credential } from '../auth-credentials/types.js';
import type { ISdk } from '../runtime/iii.js';
import type { WorkerConfig } from './config.js';
import { type AnthropicConfig, configWithCredential } from './types.js';

export async function fetchCredential(iii: ISdk): Promise<Credential> {
  const cred = await iii.trigger<unknown, Credential | null>({
    function_id: 'auth::get_token',
    payload: { provider: 'anthropic' },
    timeoutMs: 5_000,
  });
  if (!cred || typeof cred !== 'object' || !('type' in cred)) {
    throw new Error('@fn(auth::get_token) returned no credential for provider `anthropic`');
  }
  return cred;
}

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<AnthropicConfig> {
  const cred = await fetchCredential(iii);
  return configWithCredential(model, cred, worker.default_max_tokens, worker.default_api_url);
}
