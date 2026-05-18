import type { Credential } from '../auth-credentials/types.js';
import type { ISdk } from '../runtime/iii.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

export async function fetchCredential(iii: ISdk): Promise<Credential> {
  const cred = await iii.trigger<unknown, Credential | null>({
    function_id: 'auth::get_token',
    payload: { provider: 'openai' },
    timeoutMs: 5_000,
  });
  if (!cred || typeof cred !== 'object' || !('type' in cred)) {
    throw new Error('auth::get_token returned no credential for provider=openai');
  }
  return cred;
}

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<ChatCompletionsConfig> {
  const cred = await fetchCredential(iii);
  return configFromCredential(
    worker.default_api_url,
    'openai',
    model,
    cred,
    worker.default_max_tokens,
  );
}
