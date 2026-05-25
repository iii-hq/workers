import type { Credential } from '../auth-credentials/types.js';
import { fetchOverrides } from '../runtime/fetch-overrides.js';
import type { ISdk } from '../runtime/iii.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

// Look up the credential under `kimi-coding`. The compiled Rust
// auth-credentials worker (the live handler on most engines today) maps
// `kimi-coding -> MOONSHOT_API_KEY` but does not know the bare `kimi` slug
// our node port added. `kimi-coding` is the portable slug; the user-facing
// provider name on the AssistantMessage stays `kimi` (see buildConfig).
const CREDENTIAL_PROVIDER_SLUG = 'kimi-coding';

export async function fetchCredential(iii: ISdk): Promise<Credential> {
  const cred = await iii.trigger<unknown, Credential | null>({
    function_id: 'auth::get_token',
    payload: { provider: CREDENTIAL_PROVIDER_SLUG },
    timeoutMs: 5_000,
  });
  if (!cred || typeof cred !== 'object' || !('type' in cred)) {
    throw new Error(
      `auth::get_token returned no credential for provider=${CREDENTIAL_PROVIDER_SLUG} (set MOONSHOT_API_KEY)`,
    );
  }
  return cred;
}

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<ChatCompletionsConfig> {
  const [cred, overrides] = await Promise.all([
    fetchCredential(iii),
    fetchOverrides(iii, 'kimi'),
  ]);
  const apiUrl = overrides.default_api_url ?? worker.default_api_url;
  const maxTokens = overrides.default_max_tokens ?? worker.default_max_tokens;
  return configFromCredential(apiUrl, 'kimi', model, cred, maxTokens);
}
